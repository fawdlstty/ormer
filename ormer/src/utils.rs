use std::error::Error;

// ---------------------------------------------------------------------------
// 标准 Error 类型（实现了 std::error::Error）
// ---------------------------------------------------------------------------

pub trait ResultTraceExt<T> {
    #[track_caller]
    fn trace(self) -> anyhow::Result<T>;

    fn trace_for(self, func_name: &str) -> anyhow::Result<T>;
}

impl<T, E: Error> ResultTraceExt<T> for core::result::Result<T, E> {
    #[track_caller]
    fn trace(self) -> anyhow::Result<T> {
        let loc = std::panic::Location::caller();
        let func_name = format!("{}:{}", loc.file(), loc.line());
        self.trace_for(&func_name)
    }

    fn trace_for(self, func_name: &str) -> anyhow::Result<T> {
        match self {
            Ok(data) => Ok(data),
            Err(err) => bail_with_std_error(func_name, &err),
        }
    }
}

pub trait FutureTraceExt<T> {
    fn trace(self) -> impl std::future::Future<Output = anyhow::Result<T>>;

    fn trace_for(
        self,
        func_name: &'static str,
    ) -> impl std::future::Future<Output = anyhow::Result<T>>;
}

impl<T, E: Error, F: std::future::Future<Output = Result<T, E>>> FutureTraceExt<T> for F {
    fn trace(self) -> impl std::future::Future<Output = anyhow::Result<T>> {
        let type_name = std::any::type_name_of_val(&self);
        async move {
            match self.await {
                Ok(data) => Ok(data),
                Err(err) => {
                    let func_name = infer_future_func_name(type_name);
                    bail_with_std_error(&func_name, &err)
                }
            }
        }
    }

    fn trace_for(
        self,
        func_name: &'static str,
    ) -> impl std::future::Future<Output = anyhow::Result<T>> {
        async move {
            match self.await {
                Ok(data) => Ok(data),
                Err(err) => bail_with_std_error(func_name, &err),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// anyhow::Error（不实现 std::error::Error，使用 chain() 获取因果链）
// ---------------------------------------------------------------------------

pub trait AnyhowResultTraceExt<T> {
    #[track_caller]
    fn trace(self) -> anyhow::Result<T>;

    fn trace_for(self, func_name: &str) -> anyhow::Result<T>;
}

impl<T> AnyhowResultTraceExt<T> for anyhow::Result<T> {
    #[track_caller]
    fn trace(self) -> anyhow::Result<T> {
        let loc = std::panic::Location::caller();
        let func_name = format!("{}:{}", loc.file(), loc.line());
        self.trace_for(&func_name)
    }

    fn trace_for(self, func_name: &str) -> anyhow::Result<T> {
        match self {
            Ok(data) => Ok(data),
            Err(err) => bail_with_anyhow_error(func_name, &err),
        }
    }
}

pub trait AnyhowFutureTraceExt<T> {
    fn trace(self) -> impl std::future::Future<Output = anyhow::Result<T>>;

    fn trace_for(
        self,
        func_name: &'static str,
    ) -> impl std::future::Future<Output = anyhow::Result<T>>;
}

impl<T, F: std::future::Future<Output = anyhow::Result<T>>> AnyhowFutureTraceExt<T> for F {
    fn trace(self) -> impl std::future::Future<Output = anyhow::Result<T>> {
        let type_name = std::any::type_name_of_val(&self);
        async move {
            match self.await {
                Ok(data) => Ok(data),
                Err(err) => {
                    let func_name = infer_future_func_name(type_name);
                    bail_with_anyhow_error(&func_name, &err)
                }
            }
        }
    }

    fn trace_for(
        self,
        func_name: &'static str,
    ) -> impl std::future::Future<Output = anyhow::Result<T>> {
        async move {
            match self.await {
                Ok(data) => Ok(data),
                Err(err) => bail_with_anyhow_error(func_name, &err),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 内部辅助函数
// ---------------------------------------------------------------------------

fn infer_future_func_name(type_name: &str) -> String {
    let s1 = &type_name[..type_name.find("::{{closure}}").unwrap_or(type_name.len())];
    let s2 = &s1[..s1.find('<').unwrap_or(s1.len())];
    let mut parts: Vec<&str> = s2.split("::").collect();
    if parts.len() > 2 {
        parts.drain(..parts.len() - 2);
    }
    parts.join("::")
}

fn bail_with_std_error<T>(func_name: &str, err: &dyn Error) -> anyhow::Result<T> {
    let err_str = err.to_string();
    if looks_traced(&err_str) {
        anyhow::bail!("{func_name}->{err}");
    }
    // 遍历 source 链，取最深层的错误信息
    let mut deepest = err;
    while let Some(src) = deepest.source() {
        deepest = src;
    }
    if std::ptr::eq(deepest, err) {
        anyhow::bail!("{func_name} failed: {err}");
    } else {
        anyhow::bail!("{func_name} failed: {deepest}");
    }
}

fn bail_with_anyhow_error<T>(func_name: &str, err: &anyhow::Error) -> anyhow::Result<T> {
    let err_str = err.to_string();
    if looks_traced(&err_str) {
        anyhow::bail!("{func_name}->{err}");
    }
    anyhow::bail!("{func_name} failed: {err:#}");
}

fn looks_traced(err: &str) -> bool {
    let Some(prefix) = err.split(" failed: ").next() else {
        return false;
    };
    if prefix.is_empty() {
        return false;
    }
    prefix
        .split("->")
        .all(|part| part.split("::").all(is_ident_like))
}

fn is_ident_like(part: &str) -> bool {
    let mut chars = part.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}
