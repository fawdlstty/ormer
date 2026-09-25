use ormer::utils::ResultTraceExt;
use std::error::Error;
use std::fmt::{self, Display};

#[derive(Debug)]
struct DetailError(&'static str);

impl Display for DetailError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl Error for DetailError {}

#[derive(Debug)]
struct GenericDbError {
    source: DetailError,
}

impl Display for GenericDbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("db error")
    }
}

impl Error for GenericDbError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

#[test]
fn trace_for_prefers_specific_source_over_generic_db_error() {
    let err = std::io::Error::other(GenericDbError {
        source: DetailError("duplicate key value violates unique constraint"),
    });

    let traced = Err::<(), _>(err).trace_for("Database::insert").unwrap_err();

    assert_eq!(
        traced.to_string(),
        "Database::insert failed: duplicate key value violates unique constraint"
    );
}

#[test]
fn trace_for_preserves_existing_trace_prefix() {
    let traced = Err::<(), _>(DetailError("inner::task failed: boom"))
        .trace_for("outer::task")
        .unwrap_err();

    assert_eq!(traced.to_string(), "outer::task->inner::task failed: boom");
}
