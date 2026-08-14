use crate::model::Value;
use std::marker::PhantomData;
use std::ops::{AddAssign, DivAssign, MulAssign, SubAssign};

#[derive(Debug, Clone)]
pub enum UpdateValue {
    Literal(Value),
    Expr(UpdateExpr),
}

#[derive(Debug, Clone)]
pub enum UpdateExpr {
    Column(String),
    IncomingColumn(String),
    Value(Value),
    Binary {
        left: Box<UpdateExpr>,
        op: UpdateBinaryOp,
        right: Box<UpdateExpr>,
    },
    Sql(crate::query::expr::SqlExpr),
}

#[derive(Debug, Clone, Copy)]
pub enum UpdateBinaryOp {
    Add,
    Sub,
    Mul,
    Div,
}

impl UpdateBinaryOp {
    pub fn sql(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
        }
    }
}

#[derive(Debug, Clone)]
pub struct UpdateAssignment {
    pub column: String,
    pub value: UpdateValue,
}

pub trait UpdateFields {
    fn assignments(&self) -> Vec<UpdateAssignment>;
}

impl UpdateFields for () {
    fn assignments(&self) -> Vec<UpdateAssignment> {
        Vec::new()
    }
}

#[derive(Debug, Clone)]
pub struct UpdateField<T> {
    column_name: &'static str,
    assigned: bool,
    value: UpdateValue,
    _marker: PhantomData<T>,
}

impl<T> UpdateField<T> {
    pub fn new(column_name: &'static str) -> Self {
        Self {
            column_name,
            assigned: false,
            value: UpdateValue::Expr(UpdateExpr::Column(column_name.to_string())),
            _marker: PhantomData,
        }
    }

    pub fn set<V>(&self, value: V) -> Self
    where
        V: Into<Value>,
    {
        Self {
            column_name: self.column_name,
            assigned: true,
            value: UpdateValue::Literal(value.into()),
            _marker: PhantomData,
        }
    }

    pub fn incoming(&self) -> Self {
        Self {
            column_name: self.column_name,
            assigned: true,
            value: UpdateValue::Expr(UpdateExpr::IncomingColumn(self.column_name.to_string())),
            _marker: PhantomData,
        }
    }

    pub fn json_set<V>(&self, path: impl crate::query::builder::IntoJsonPath, value: V) -> Self
    where
        V: Into<Value>,
    {
        Self {
            column_name: self.column_name,
            assigned: true,
            value: UpdateValue::Expr(UpdateExpr::Sql(crate::query::expr::SqlExpr::JsonSet {
                expr: Box::new(crate::query::expr::SqlExpr::Column(
                    self.column_name.to_string(),
                )),
                path: path.into_json_path(),
                value: Box::new(crate::query::expr::SqlExpr::Value(value.into())),
            })),
            _marker: PhantomData,
        }
    }

    pub fn array_append<V>(&self, value: V) -> Self
    where
        V: Into<Value>,
    {
        Self {
            column_name: self.column_name,
            assigned: true,
            value: UpdateValue::Expr(UpdateExpr::Sql(crate::query::expr::SqlExpr::Function {
                name: "array_append",
                args: vec![
                    crate::query::expr::SqlExpr::Column(self.column_name.to_string()),
                    crate::query::expr::SqlExpr::Value(value.into()),
                ],
            })),
            _marker: PhantomData,
        }
    }

    pub fn array_remove<V>(&self, value: V) -> Self
    where
        V: Into<Value>,
    {
        Self {
            column_name: self.column_name,
            assigned: true,
            value: UpdateValue::Expr(UpdateExpr::Sql(crate::query::expr::SqlExpr::Function {
                name: "array_remove",
                args: vec![
                    crate::query::expr::SqlExpr::Column(self.column_name.to_string()),
                    crate::query::expr::SqlExpr::Value(value.into()),
                ],
            })),
            _marker: PhantomData,
        }
    }

    pub fn excluded(&self) -> Self {
        self.incoming()
    }

    pub fn assignment(&self) -> Option<UpdateAssignment> {
        self.assigned.then(|| UpdateAssignment {
            column: self.column_name.to_string(),
            value: self.value.clone(),
        })
    }

    fn apply_binary<V>(&mut self, op: UpdateBinaryOp, value: V)
    where
        V: Into<Value>,
    {
        self.assigned = true;
        self.value = UpdateValue::Expr(UpdateExpr::Binary {
            left: Box::new(UpdateExpr::Column(self.column_name.to_string())),
            op,
            right: Box::new(UpdateExpr::Value(value.into())),
        });
    }
}

impl<T, V> AddAssign<V> for UpdateField<T>
where
    V: Into<Value>,
{
    fn add_assign(&mut self, rhs: V) {
        self.apply_binary(UpdateBinaryOp::Add, rhs);
    }
}

impl<T, V> SubAssign<V> for UpdateField<T>
where
    V: Into<Value>,
{
    fn sub_assign(&mut self, rhs: V) {
        self.apply_binary(UpdateBinaryOp::Sub, rhs);
    }
}

impl<T, V> MulAssign<V> for UpdateField<T>
where
    V: Into<Value>,
{
    fn mul_assign(&mut self, rhs: V) {
        self.apply_binary(UpdateBinaryOp::Mul, rhs);
    }
}

impl<T, V> DivAssign<V> for UpdateField<T>
where
    V: Into<Value>,
{
    fn div_assign(&mut self, rhs: V) {
        self.apply_binary(UpdateBinaryOp::Div, rhs);
    }
}
