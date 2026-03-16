use anki_api_proto::anki::api::v1::ErrorDetail;
use prost_types::Timestamp;
use prost14::Message;
#[cfg(test)]
use std::fs;
#[cfg(test)]
use std::path::PathBuf;
#[cfg(test)]
use std::time::{SystemTime, UNIX_EPOCH};
use tonic::Code;
use tonic::Status;

const DEFAULT_CHANGE_LIMIT: usize = 200;
const MAX_CHANGE_LIMIT: usize = 1000;
type ChangePage = (Vec<(i64, i64, i64)>, String);

pub fn parse_usn_cursor(cursor: &str) -> Result<(i64, i64), Status> {
    if cursor.is_empty() {
        return Ok((i64::MIN, i64::MIN));
    }

    let Some((usn, id)) = cursor.split_once(':') else {
        return Err(Status::invalid_argument(
            "cursor must have format '<usn>:<id>'",
        ));
    };
    let usn = usn
        .parse::<i64>()
        .map_err(|_| Status::invalid_argument("cursor usn must be an integer"))?;
    let id = id
        .parse::<i64>()
        .map_err(|_| Status::invalid_argument("cursor id must be an integer"))?;
    Ok((usn, id))
}

pub fn format_usn_cursor(usn: i64, id: i64) -> String {
    format!("{usn}:{id}")
}

pub fn normalize_change_limit(limit: u32) -> usize {
    if limit == 0 {
        return DEFAULT_CHANGE_LIMIT;
    }
    (limit as usize).min(MAX_CHANGE_LIMIT)
}

pub fn timestamp_from_secs(seconds: i64) -> Timestamp {
    Timestamp { seconds, nanos: 0 }
}

pub fn enforce_expected_usn(
    expected_usn: Option<i64>,
    actual_usn: i64,
    resource: &str,
    resource_id: i64,
) -> Result<(), Status> {
    // Optimistic concurrency contract for write RPCs:
    // - if `expected_usn` is provided and does not match current usn,
    //   return ABORTED with a stable VERSION_CONFLICT marker.
    // - if `expected_usn` is omitted, writes keep last-writer-wins behavior.
    if let Some(expected_usn) = expected_usn
        && expected_usn != actual_usn
    {
        let detail = ErrorDetail {
            code: "VERSION_CONFLICT".to_owned(),
            retryable: true,
            message: "write precondition did not match current usn".to_owned(),
        };
        let message = format!(
            "version conflict for {resource} id={resource_id}: expected_usn={expected_usn} actual_usn={actual_usn}"
        );
        return Err(Status::with_details(
            Code::Aborted,
            message,
            detail.encode_to_vec().into(),
        ));
    }
    Ok(())
}

pub fn annotate_batch_status(status: Status, operation: &str, index: usize) -> Status {
    let message = format!(
        "{} failed at batch index {}: {}",
        operation,
        index,
        status.message()
    );
    Status::with_details_and_metadata(
        status.code(),
        message,
        status.details().to_vec().into(),
        status.metadata().clone(),
    )
}

pub fn execute_batch<I, R, F>(
    items: Vec<I>,
    operation: &str,
    mut process: F,
) -> Result<Vec<R>, Status>
where
    F: FnMut(I) -> Result<R, Status>,
{
    let mut results = Vec::with_capacity(items.len());
    for (index, item) in items.into_iter().enumerate() {
        let result =
            process(item).map_err(|status| annotate_batch_status(status, operation, index))?;
        results.push(result);
    }
    Ok(results)
}

pub fn get_changes_page<F>(
    cursor: &str,
    limit: u32,
    mut fetch_rows: F,
) -> Result<ChangePage, Status>
where
    F: FnMut((i64, i64), u32) -> Result<Vec<(i64, i64, i64)>, Status>,
{
    let cursor = parse_usn_cursor(cursor)?;
    let limit = normalize_change_limit(limit);
    let fetch_limit =
        u32::try_from(limit.saturating_add(1)).map_err(|_| Status::internal("invalid limit"))?;
    let mut rows = fetch_rows(cursor, fetch_limit)?;
    let has_more = rows.len() > limit;
    if has_more {
        rows.truncate(limit);
    }
    let next_cursor = if has_more {
        if let Some((usn, id, _)) = rows.last() {
            format_usn_cursor(*usn, *id)
        } else {
            String::new()
        }
    } else {
        String::new()
    };
    Ok((rows, next_cursor))
}

#[cfg(test)]
pub(crate) struct TestStore {
    root: PathBuf,
    store: Option<crate::store::SharedStore>,
}

#[cfg(test)]
impl TestStore {
    pub(crate) fn new(test_name: &str) -> Self {
        let mut root = std::env::temp_dir();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        root.push(format!("anki-api-test-{test_name}-{nanos}"));
        fs::create_dir_all(&root).expect("create temp test dir");
        let collection_path = root.join("collection.anki2");
        let store =
            crate::store::initialize_store_for_test(collection_path).expect("initialize store");
        Self {
            root,
            store: Some(store),
        }
    }

    pub(crate) fn store(&self) -> crate::store::SharedStore {
        self.store.as_ref().expect("store initialized").clone()
    }
}

#[cfg(test)]
impl Drop for TestStore {
    fn drop(&mut self) {
        // Ensure backend/store drops before filesystem cleanup.
        let _ = self.store.take();
        let _ = fs::remove_dir_all(&self.root);
    }
}
