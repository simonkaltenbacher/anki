// Copyright: Ankitects Pty Ltd and contributors
// License: GNU AGPL, version 3 or later; http://www.gnu.org/licenses/agpl.html

use anki::backend::init_backend;
use anki::backend::Backend as RustBackend;
use anki::log::set_global_logger;
use anki::sync::http_server::SimpleServer;
use anki_api::config::FileConfig;
use anki_api::config::RuntimeOverrides;
use anki_api::config::ServerConfig;
use anki_api::grpc;
use anki_api::logging;
use anki_api::store;
use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use pyo3::types::PyDict;
use pyo3::wrap_pyfunction;
use pyo3::FromPyObject;
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
    fn api_server_file_config_status(&self) -> PyResult<(Option<bool>, bool)> {
        let file_config =
            FileConfig::load_default().map_err(|err| PyException::new_err(err.to_string()))?;
        Ok((file_config.enabled, file_config.has_runtime_fields_set()))
    }

    #[pyo3(signature = (*, runtime_overrides=None))]
    fn start_api_server(&self, runtime_overrides: Option<&Bound<'_, PyDict>>) -> PyResult<()> {
        let mut guard = self
            .api_server
            .lock()
            .map_err(|_| PyException::new_err("api server mutex poisoned"))?;
        if guard.is_some() {
            return Err(PyException::new_err("anki api server is already running"));
        }

        let file_config =
            FileConfig::load_default().map_err(|err| PyException::new_err(err.to_string()))?;
        let runtime_overrides = parse_runtime_overrides(runtime_overrides)?;
        let config = ServerConfig::resolve(runtime_overrides, file_config)
            .map_err(|err| PyException::new_err(err.to_string()))?;
        logging::init_stderr_logging();

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

fn extract_optional<'a, T>(dict: Option<&Bound<'a, PyDict>>, key: &str) -> PyResult<Option<T>>
where
    T: FromPyObject<'a>,
{
    match dict {
        Some(dict) => dict
            .get_item(key)?
            .map(|value| {
                if value.is_none() {
                    Ok(None)
                } else {
                    value.extract().map(Some)
                }
            })
            .transpose()
            .map(Option::flatten),
        None => Ok(None),
    }
}

fn extract_string(dict: Option<&Bound<'_, PyDict>>, key: &str) -> PyResult<Option<String>> {
    extract_optional(dict, key)
}

fn extract_u16(dict: Option<&Bound<'_, PyDict>>, key: &str) -> PyResult<Option<u16>> {
    extract_optional(dict, key)
}

fn extract_bool(dict: Option<&Bound<'_, PyDict>>, key: &str) -> PyResult<Option<bool>> {
    extract_optional(dict, key)
}

fn parse_runtime_overrides(dict: Option<&Bound<'_, PyDict>>) -> PyResult<RuntimeOverrides> {
    Ok(RuntimeOverrides {
        host: extract_string(dict, "host")?,
        port: extract_u16(dict, "port")?,
        api_key: extract_string(dict, "api_key")?,
        anki_version: extract_string(dict, "anki_version")?,
        auth_disabled: extract_bool(dict, "auth_disabled")?,
        allow_non_local: extract_bool(dict, "allow_non_local")?,
        allow_loopback_unauthenticated_health_check: extract_bool(
            dict,
            "allow_loopback_unauthenticated_health_check",
        )?,
        transport_mode: extract_string(dict, "transport_mode")?,
        tls_cert_path: extract_string(dict, "tls_cert_path")?,
        tls_key_path: extract_string(dict, "tls_key_path")?,
        spiffe_allowed_client_id: extract_string(dict, "spiffe_allowed_client_id")?,
        spiffe_workload_api_socket: extract_string(dict, "spiffe_workload_api_socket")?,
    })
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
