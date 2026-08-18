use crate::OrmerError;
use crate::model::Value;
use std::error::Error;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, Instant};

type BeforeCallback = Arc<dyn Fn(&SqlTraceEvent) + Send + Sync + 'static>;
type AfterCallback = Arc<dyn Fn(&SqlTraceEvent, Duration) + Send + Sync + 'static>;
type ErrorCallback = Arc<dyn Fn(&SqlTraceEvent, Duration, &OrmerError) + Send + Sync + 'static>;
type RewriteCallback = Arc<dyn Fn(&SqlTraceEvent) -> String + Send + Sync + 'static>;
type ParamsRedactor = Arc<dyn Fn(&[Value]) -> Vec<Value> + Send + Sync + 'static>;

static GLOBAL_SQL_TRACE: OnceLock<SqlTrace> = OnceLock::new();

#[derive(Clone)]
pub struct SqlTrace {
    inner: Arc<RwLock<SqlTraceState>>,
}

impl Default for SqlTrace {
    fn default() -> Self {
        Self {
            inner: Arc::new(RwLock::new(SqlTraceState::default())),
        }
    }
}

#[derive(Default)]
struct SqlTraceState {
    before: Vec<BeforeCallback>,
    after: Vec<AfterCallback>,
    on_error: Vec<ErrorCallback>,
    slow: Vec<AfterCallback>,
    slow_threshold: Option<Duration>,
    rewrite: Option<RewriteCallback>,
    params_redactor: Option<ParamsRedactor>,
}

#[derive(Clone)]
struct SqlTraceSnapshot {
    before: Vec<BeforeCallback>,
    after: Vec<AfterCallback>,
    on_error: Vec<ErrorCallback>,
    slow: Vec<AfterCallback>,
    slow_threshold: Option<Duration>,
    rewrite: Option<RewriteCallback>,
    params_redactor: Option<ParamsRedactor>,
}

#[derive(Debug, Clone)]
pub struct SqlTraceEvent {
    sql: String,
    params: Vec<Value>,
}

impl SqlTraceEvent {
    pub fn sql(&self) -> &str {
        &self.sql
    }

    pub fn params(&self) -> &[Value] {
        &self.params
    }
}

#[derive(Clone)]
pub struct SqlTraceBuilder {
    trace: SqlTrace,
}

impl SqlTraceBuilder {
    pub fn clear(self) -> Self {
        self.trace
            .with_state(|state| *state = SqlTraceState::default());
        self
    }

    pub fn before<F>(self, callback: F) -> Self
    where
        F: Fn(&str) + Send + Sync + 'static,
    {
        self.before_with(move |event| callback(event.sql()))
    }

    pub fn before_with<F>(self, callback: F) -> Self
    where
        F: Fn(&SqlTraceEvent) + Send + Sync + 'static,
    {
        self.trace
            .with_state(|state| state.before.push(Arc::new(callback)));
        self
    }

    pub fn after<F>(self, callback: F) -> Self
    where
        F: Fn(&str, Duration) + Send + Sync + 'static,
    {
        self.after_with(move |event, elapsed| callback(event.sql(), elapsed))
    }

    pub fn after_with<F>(self, callback: F) -> Self
    where
        F: Fn(&SqlTraceEvent, Duration) + Send + Sync + 'static,
    {
        self.trace
            .with_state(|state| state.after.push(Arc::new(callback)));
        self
    }

    pub fn on_error<F>(self, callback: F) -> Self
    where
        F: Fn(&str, &OrmerError) + Send + Sync + 'static,
    {
        self.on_error_with(move |event, _elapsed, error| callback(event.sql(), error))
    }

    pub fn on_error_with<F>(self, callback: F) -> Self
    where
        F: Fn(&SqlTraceEvent, Duration, &OrmerError) + Send + Sync + 'static,
    {
        self.trace
            .with_state(|state| state.on_error.push(Arc::new(callback)));
        self
    }

    pub fn slow_sql_threshold(self, threshold: Duration) -> Self {
        self.trace
            .with_state(|state| state.slow_threshold = Some(threshold));
        self
    }

    pub fn slow<F>(self, callback: F) -> Self
    where
        F: Fn(&str, Duration) + Send + Sync + 'static,
    {
        self.slow_with(move |event, elapsed| callback(event.sql(), elapsed))
    }

    pub fn slow_with<F>(self, callback: F) -> Self
    where
        F: Fn(&SqlTraceEvent, Duration) + Send + Sync + 'static,
    {
        self.trace
            .with_state(|state| state.slow.push(Arc::new(callback)));
        self
    }

    pub fn rewrite<F>(self, callback: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        self.rewrite_with(move |event| callback(event.sql()))
    }

    pub fn rewrite_with<F>(self, callback: F) -> Self
    where
        F: Fn(&SqlTraceEvent) -> String + Send + Sync + 'static,
    {
        self.trace
            .with_state(|state| state.rewrite = Some(Arc::new(callback)));
        self
    }

    pub fn redact_params<F>(self, callback: F) -> Self
    where
        F: Fn(&[Value]) -> Vec<Value> + Send + Sync + 'static,
    {
        self.trace
            .with_state(|state| state.params_redactor = Some(Arc::new(callback)));
        self
    }
}

pub(crate) struct SqlTraceExecution {
    sql: String,
    event: SqlTraceEvent,
    started_at: Instant,
    snapshot: SqlTraceSnapshot,
}

impl SqlTrace {
    pub fn builder(&self) -> SqlTraceBuilder {
        SqlTraceBuilder {
            trace: self.clone(),
        }
    }

    fn with_state(&self, f: impl FnOnce(&mut SqlTraceState)) {
        let mut state = self.inner.write().unwrap_or_else(|err| err.into_inner());
        f(&mut state);
    }

    fn snapshot(&self) -> SqlTraceSnapshot {
        let state = self.inner.read().unwrap_or_else(|err| err.into_inner());
        SqlTraceSnapshot {
            before: state.before.clone(),
            after: state.after.clone(),
            on_error: state.on_error.clone(),
            slow: state.slow.clone(),
            slow_threshold: state.slow_threshold,
            rewrite: state.rewrite.clone(),
            params_redactor: state.params_redactor.clone(),
        }
    }

    fn start(&self, sql: &str, params: &[Value]) -> SqlTraceExecution {
        let snapshot = self.snapshot();
        let rewrite_event = SqlTraceEvent {
            sql: sql.to_string(),
            params: params.to_vec(),
        };
        let sql = snapshot
            .rewrite
            .as_ref()
            .map(|callback| callback(&rewrite_event))
            .unwrap_or_else(|| sql.to_string());
        let params = snapshot
            .params_redactor
            .as_ref()
            .map(|redactor| redactor(params))
            .unwrap_or_else(|| params.to_vec());
        let event = SqlTraceEvent {
            sql: sql.clone(),
            params,
        };

        for callback in &snapshot.before {
            callback(&event);
        }

        SqlTraceExecution {
            sql,
            event,
            started_at: Instant::now(),
            snapshot,
        }
    }
}

impl SqlTraceExecution {
    pub(crate) fn sql(&self) -> &str {
        &self.sql
    }

    pub(crate) fn finish_ok(self) {
        let elapsed = self.started_at.elapsed();
        for callback in &self.snapshot.after {
            callback(&self.event, elapsed);
        }
        if self
            .snapshot
            .slow_threshold
            .map(|threshold| elapsed >= threshold)
            .unwrap_or(false)
        {
            for callback in &self.snapshot.slow {
                callback(&self.event, elapsed);
            }
        }
    }

    pub(crate) fn finish_error(self, error: OrmerError) -> OrmerError {
        let elapsed = self.started_at.elapsed();
        for callback in &self.snapshot.on_error {
            callback(&self.event, elapsed, &error);
        }
        error
    }

    pub(crate) fn finish_external_error<E: Error>(self, context: &str, error: E) -> OrmerError {
        self.finish_error(OrmerError::from_external(context, error))
    }
}

pub(crate) fn start_sql_trace(sql: &str, params: &[Value]) -> SqlTraceExecution {
    global_sql_trace().start(sql, params)
}

pub fn global_sql_trace() -> &'static SqlTrace {
    GLOBAL_SQL_TRACE.get_or_init(SqlTrace::default)
}
