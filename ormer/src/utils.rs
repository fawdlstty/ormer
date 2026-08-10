use crate::{OrmerError, Result};
use std::error::Error;

pub trait ResultTraceExt<T> {
    #[track_caller]
    fn trace(self) -> Result<T>;

    fn trace_for(self, context: &str) -> Result<T>;
}

impl<T, E: Error> ResultTraceExt<T> for std::result::Result<T, E> {
    #[track_caller]
    fn trace(self) -> Result<T> {
        let location = std::panic::Location::caller();
        self.trace_for(&format!("{}:{}", location.file(), location.line()))
    }

    fn trace_for(self, context: &str) -> Result<T> {
        self.map_err(|error| OrmerError::from_external(context, error))
    }
}

pub trait FutureTraceExt<T> {
    fn trace(self) -> impl std::future::Future<Output = Result<T>>;

    fn trace_for(self, context: &str) -> impl std::future::Future<Output = Result<T>>;
}

impl<T, E: Error, F: std::future::Future<Output = std::result::Result<T, E>>> FutureTraceExt<T>
    for F
{
    fn trace(self) -> impl std::future::Future<Output = Result<T>> {
        let type_name = std::any::type_name_of_val(&self).to_string();
        async move {
            self.await.map_err(|error| {
                OrmerError::from_external(&infer_future_func_name(&type_name), error)
            })
        }
    }

    fn trace_for(self, context: &str) -> impl std::future::Future<Output = Result<T>> {
        let context = context.to_string();
        async move {
            self.await
                .map_err(|error| OrmerError::from_external(&context, error))
        }
    }
}

fn infer_future_func_name(type_name: &str) -> String {
    let closure = &type_name[..type_name.find("::{{closure}}").unwrap_or(type_name.len())];
    let type_name = &closure[..closure.find('<').unwrap_or(closure.len())];
    let mut parts: Vec<&str> = type_name.split("::").collect();
    if parts.len() > 2 {
        parts.drain(..parts.len() - 2);
    }
    parts.join("::")
}
