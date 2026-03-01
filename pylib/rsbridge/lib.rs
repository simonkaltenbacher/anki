// Copyright: Ankitects Pty Ltd and contributors
// License: GNU AGPL, version 3 or later; http://www.gnu.org/licenses/agpl.html

use anki::backend::init_backend;
use anki::backend::Backend as RustBackend;
use anki::log::set_global_logger;
use anki::sync::http_server::SimpleServer;
use anki_api::config::ProfileConfig;
use anki_api::config::RuntimeOverrides;
use anki_api::config::ServerConfig;
use anki_api::grpc;
use anki_api::store;
use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use pyo3::wrap_pyfunction;
use std::sync::mpsc;
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::Duration;
use tokio::sync::oneshot;

#[pyclass(module = "_rsbridge")]
struct Backend {
    backend: RustBackend,
    api_server: Mutex<Option<ApiServerHandle>>,
}

struct ApiServerHandle {
    shutdown_tx: oneshot::Sender<()>,
    join_handle: JoinHandle<()>,
}

create_exception!(_rsbridge, BackendError, PyException);

#[pyfunction]
fn buildhash() -> &'static str {
    anki::version::buildhash()
}

#[pyfunction]
#[pyo3(signature = (path=None))]
fn initialize_logging(path: Option<&str>) -> PyResult<()> {
    set_global_logger(path).map_err(|e| PyException::new_err(e.to_string()))
}

#[pyfunction]
fn syncserver() -> PyResult<()> {
    set_global_logger(None).unwrap();
    let err = SimpleServer::run();
    Err(PyException::new_err(err.to_string()))
}

#[pyfunction]
fn open_backend(init_msg: &Bound<'_, PyBytes>) -> PyResult<Backend> {
    match init_backend(init_msg.as_bytes()) {
        Ok(backend) => Ok(Backend {
            backend,
            api_server: Mutex::new(None),
        }),
        Err(e) => Err(PyException::new_err(e)),
    }
}

#[pymethods]
impl Backend {
    #[pyo3(signature = (
        host=None,
        port=None,
        api_key=None,
        anki_version=None,
        auth_disabled=None,
        allow_non_local=None,
        allow_loopback_unauthenticated_health_check=None
    ))]
    fn start_api_server(
        &self,
        host: Option<String>,
        port: Option<u16>,
        api_key: Option<String>,
        anki_version: Option<String>,
        auth_disabled: Option<bool>,
        allow_non_local: Option<bool>,
        allow_loopback_unauthenticated_health_check: Option<bool>,
    ) -> PyResult<()> {
        let mut guard = self
            .api_server
            .lock()
            .map_err(|_| PyException::new_err("api server mutex poisoned"))?;
        if guard.is_some() {
            return Err(PyException::new_err("anki api server is already running"));
        }

        let config = ServerConfig::resolve(
            RuntimeOverrides {
                host,
                port,
                api_key,
                anki_version,
                auth_disabled,
                allow_non_local,
                allow_loopback_unauthenticated_health_check,
            },
            ProfileConfig::default(),
        )
        .map_err(|err| PyException::new_err(err.to_string()))?;

        let store = store::shared_store_from_backend(self.backend.clone());
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let join_handle = std::thread::Builder::new()
            .name("anki-api-grpc".to_owned())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(err) => {
                        let _ = ready_tx.send(Err(format!(
                            "failed to build tokio runtime for anki api server: {err}"
                        )));
                        eprintln!("failed to build tokio runtime for anki api server: {err}");
                        return;
                    }
                };

                runtime.block_on(async move {
                    if let Err(err) = grpc::serve_with_store_and_shutdown_and_ready(
                        config,
                        store,
                        async move {
                            let _ = shutdown_rx.await;
                        },
                        Some(ready_tx),
                    )
                    .await
                    {
                        eprintln!("anki api server terminated with error: {err}");
                    }
                });
            })
            .map_err(|err| PyException::new_err(format!("failed to spawn api server: {err}")))?;

        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                let _ = shutdown_tx.send(());
                let _ = join_handle.join();
                return Err(PyException::new_err(err));
            }
            Err(err) => {
                let _ = shutdown_tx.send(());
                let _ = join_handle.join();
                return Err(PyException::new_err(format!(
                    "api server did not signal readiness: {err}"
                )));
            }
        }

        *guard = Some(ApiServerHandle {
            shutdown_tx,
            join_handle,
        });
        Ok(())
    }

    fn stop_api_server(&self) -> PyResult<()> {
        stop_api_server_impl(&self.api_server)
    }

    fn command(
        &self,
        py: Python,
        service: u32,
        method: u32,
        input: &Bound<'_, PyBytes>,
    ) -> PyResult<PyObject> {
        let in_bytes = input.as_bytes();
        py.allow_threads(|| self.backend.run_service_method(service, method, in_bytes))
            .map(|out_bytes| {
                let out_obj = PyBytes::new(py, &out_bytes);
                out_obj.into()
            })
            .map_err(BackendError::new_err)
    }

    /// This takes and returns JSON, due to Python's slow protobuf
    /// encoding/decoding.
    fn db_command(&self, py: Python, input: &Bound<'_, PyBytes>) -> PyResult<PyObject> {
        let in_bytes = input.as_bytes();
        let out_res = py.allow_threads(|| {
            self.backend
                .run_db_command_bytes(in_bytes)
                .map_err(BackendError::new_err)
        });
        let out_bytes = out_res?;
        let out_obj = PyBytes::new(py, &out_bytes);
        Ok(out_obj.into())
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        let _ = stop_api_server_impl(&self.api_server);
    }
}

fn stop_api_server_impl(api_server: &Mutex<Option<ApiServerHandle>>) -> PyResult<()> {
    let handle = {
        let mut guard = api_server
            .lock()
            .map_err(|_| PyException::new_err("api server mutex poisoned"))?;
        guard.take()
    };

    if let Some(handle) = handle {
        let _ = handle.shutdown_tx.send(());
        if let Err(err) = handle.join_handle.join() {
            return Err(PyException::new_err(format!(
                "failed to join api server thread: {err:?}"
            )));
        }
    }

    Ok(())
}

// Module definition
//////////////////////////////////

#[pymodule]
fn _rsbridge(_py: Python, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Backend>()?;
    m.add_wrapped(wrap_pyfunction!(buildhash)).unwrap();
    m.add_wrapped(wrap_pyfunction!(open_backend)).unwrap();
    m.add_wrapped(wrap_pyfunction!(initialize_logging)).unwrap();
    m.add_wrapped(wrap_pyfunction!(syncserver)).unwrap();

    Ok(())
}
