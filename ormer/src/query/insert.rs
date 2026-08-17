use crate::model::{Model, Value};
use crate::query::builder::{TypedColumn, WhereExpr};
use crate::query::filter::FilterExpr;
use crate::query::update::UpdateAssignment;

#[derive(Debug, Clone)]
pub enum InsertConflictTarget {
    Columns(Vec<&'static str>),
    Constraint(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertConflictAction {
    DoNothing,
    DoUpdate,
}

#[derive(Debug, Clone, Default)]
pub struct InsertConflict {
    pub target: Option<InsertConflictTarget>,
    pub target_filter: Option<FilterExpr>,
    pub action: Option<InsertConflictAction>,
    pub update_filter: Option<FilterExpr>,
    pub assignments: Vec<UpdateAssignment>,
}

#[derive(Debug, Clone)]
pub enum InsertValue {
    Literal(Value),
    Default,
}

#[derive(Debug, Clone)]
pub struct InsertAssignment {
    pub column: String,
    pub value: InsertValue,
}

impl InsertAssignment {
    pub fn value(column: impl Into<String>, value: impl Into<Value>) -> Self {
        Self {
            column: column.into(),
            value: InsertValue::Literal(value.into()),
        }
    }

    pub fn default(column: impl Into<String>) -> Self {
        Self {
            column: column.into(),
            value: InsertValue::Default,
        }
    }
}

pub trait IntoInsertAssignment<M: Model> {
    fn into_insert_assignment(self) -> InsertAssignment;
}

impl<M: Model> IntoInsertAssignment<M> for InsertAssignment {
    fn into_insert_assignment(self) -> InsertAssignment {
        self
    }
}

impl<M, T, S, V> IntoInsertAssignment<M> for (TypedColumn<T, S>, V)
where
    M: Model,
    V: Into<Value>,
{
    fn into_insert_assignment(self) -> InsertAssignment {
        InsertAssignment::value(self.0.column_name(), self.1)
    }
}

pub trait IntoInsertDefaultColumn<M: Model> {
    fn into_insert_default_column(self) -> String;
}

impl<M, T, S> IntoInsertDefaultColumn<M> for TypedColumn<T, S>
where
    M: Model,
{
    fn into_insert_default_column(self) -> String {
        self.column_name().to_string()
    }
}

impl<M: Model> IntoInsertDefaultColumn<M> for &'static str {
    fn into_insert_default_column(self) -> String {
        self.to_string()
    }
}

impl<M: Model> IntoInsertDefaultColumn<M> for String {
    fn into_insert_default_column(self) -> String {
        self
    }
}

impl<T, S> TypedColumn<T, S> {
    pub fn set<V>(self, value: V) -> InsertAssignment
    where
        V: Into<Value>,
    {
        InsertAssignment::value(self.column_name(), value)
    }
}

impl InsertConflict {
    pub fn is_configured(&self) -> bool {
        self.target.is_some()
            || self.target_filter.is_some()
            || self.action.is_some()
            || self.update_filter.is_some()
            || !self.assignments.is_empty()
    }
}

pub trait ConflictColumns {
    fn conflict_columns(self) -> Vec<&'static str>;
}

impl<T, S> ConflictColumns for TypedColumn<T, S> {
    fn conflict_columns(self) -> Vec<&'static str> {
        vec![self.column_name()]
    }
}

macro_rules! impl_conflict_columns_for_tuple {
    ($($type:ident => $value:ident),+) => {
        impl<$($type),+> ConflictColumns for ($($type,)+)
        where
            $($type: ConflictColumns),+
        {
            fn conflict_columns(self) -> Vec<&'static str> {
                let ($($value,)+) = self;
                let mut columns = Vec::new();
                $(columns.extend($value.conflict_columns());)+
                columns
            }
        }
    };
}

impl_conflict_columns_for_tuple!(A => a, B => b);
impl_conflict_columns_for_tuple!(A => a, B => b, C => c);
impl_conflict_columns_for_tuple!(A => a, B => b, C => c, D => d);

pub trait IntoInsertConflictTarget<M: Model> {
    fn into_insert_conflict_target(self) -> InsertConflictTarget;
}

impl<M, F, C> IntoInsertConflictTarget<M> for F
where
    M: Model,
    F: FnOnce(M::Where) -> C,
    C: ConflictColumns,
{
    fn into_insert_conflict_target(self) -> InsertConflictTarget {
        InsertConflictTarget::Columns(self(M::Where::default()).conflict_columns())
    }
}

impl<M: Model> IntoInsertConflictTarget<M> for &str {
    fn into_insert_conflict_target(self) -> InsertConflictTarget {
        InsertConflictTarget::Constraint(self.to_string())
    }
}

impl<M: Model> IntoInsertConflictTarget<M> for String {
    fn into_insert_conflict_target(self) -> InsertConflictTarget {
        InsertConflictTarget::Constraint(self)
    }
}

pub fn where_expr_to_filter<W>(expr: W) -> FilterExpr
where
    W: Into<WhereExpr>,
{
    expr.into().into()
}
