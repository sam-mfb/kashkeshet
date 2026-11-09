//! The questions a store can ask: [`Select`], [`Count`] and [`Group`],
//! built from [`Condition`]s on [`Operand`]s.
//!
//! The language is narrow on purpose: it holds what kkt's stores need and
//! nothing more, so that every backend can answer it the same way.
//!
//! **Logic is two-valued.** A comparison involving a null, on either side,
//! is false, and [`Condition::Not`] of a false comparison is true. So
//! `Not(Eq(owner, "amber"))` holds for a row whose owner is null.
//!
//! **Text** compares and orders by Unicode code point (byte order of
//! UTF-8). **Ordering** puts nulls first when ascending, last when
//! descending.

use crate::Value;

/// Something a condition or field reads from a row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operand {
    /// A column of the row.
    Column(&'static str),
    /// A column of the row that a reference column points to: `reference`
    /// is a column of this row declared with [`crate::Column::references`],
    /// and `column` a column of the table it references. Null when the
    /// reference is null or holds its exception value.
    Through {
        reference: &'static str,
        column: &'static str,
    },
}

/// Shorthand for [`Operand::Column`].
pub fn col(name: &'static str) -> Operand {
    Operand::Column(name)
}

/// Shorthand for [`Operand::Through`].
pub fn through(reference: &'static str, column: &'static str) -> Operand {
    Operand::Through { reference, column }
}

/// The rows of another table tied to this one: those of `table` whose
/// column `column` equals this row's column `to`, and that meet `filter`.
/// When this row's `to` is null, there are none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Related {
    pub table: &'static str,
    pub column: &'static str,
    pub to: &'static str,
    /// Evaluated on the related rows.
    pub filter: Condition,
}

impl Related {
    /// The rows of `table` whose `column` equals this row's `to`.
    pub fn new(table: &'static str, column: &'static str, to: &'static str) -> Related {
        Related {
            table,
            column,
            to,
            filter: Condition::True,
        }
    }

    /// Only those related rows that also meet `filter`.
    pub fn filter(mut self, filter: Condition) -> Related {
        self.filter = filter;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Condition {
    /// Every row.
    True,
    Eq(Operand, Value),
    Ne(Operand, Value),
    Lt(Operand, Value),
    Le(Operand, Value),
    Gt(Operand, Value),
    Ge(Operand, Value),
    IsNull(Operand),
    NotNull(Operand),
    /// The operand equals one of the values. False for an empty list.
    OneOf(Operand, Vec<Value>),
    /// Text equal to this, ignoring the case of ASCII letters only.
    EqIgnoringAsciiCase(Operand, String),
    /// Text containing this text, literally, ignoring the case of ASCII
    /// letters only. No character is a wildcard.
    ContainsIgnoringAsciiCase(Operand, String),
    /// Some related row exists.
    Exists(Box<Related>),
    /// Every condition holds. True when there are none.
    All(Vec<Condition>),
    /// Some condition holds. False when there are none.
    Any(Vec<Condition>),
    Not(Box<Condition>),
}

impl Condition {
    pub fn exists(related: Related) -> Condition {
        Condition::Exists(Box::new(related))
    }

    #[allow(clippy::should_implement_trait)]
    pub fn not(condition: Condition) -> Condition {
        Condition::Not(Box::new(condition))
    }

    /// This condition and `other`.
    pub fn and(self, other: Condition) -> Condition {
        match self {
            Condition::True => other,
            Condition::All(mut all) => {
                all.push(other);
                Condition::All(all)
            }
            first => Condition::All(vec![first, other]),
        }
    }
}

/// A value a [`Select`] returns for each row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Field {
    Value(Operand),
    /// How many related rows there are.
    Count(Related),
    /// The largest value of `column` among the related rows; null when
    /// there are none.
    Max(Related, &'static str),
    /// The value of `column` in the related row with the largest `by`;
    /// null when there are none. `by` identifies a related row.
    Newest {
        related: Related,
        by: &'static str,
        column: &'static str,
    },
    /// Whether any related row exists.
    Exists(Related),
}

/// What rows are ordered by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OrderBy {
    Operand(Operand),
    /// The first operand, or the second where the first is null.
    FirstOf(Operand, Operand),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sort {
    Asc(OrderBy),
    Desc(OrderBy),
}

impl Sort {
    pub fn asc(operand: Operand) -> Sort {
        Sort::Asc(OrderBy::Operand(operand))
    }

    pub fn desc(operand: Operand) -> Sort {
        Sort::Desc(OrderBy::Operand(operand))
    }
}

/// Rows of one table: for each row that meets `filter`, the `fields`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Select {
    pub table: &'static str,
    pub fields: Vec<Field>,
    pub filter: Condition,
    /// In this order. Rows that tie on every sort come in an order the
    /// backend chooses, so a store that pages sorts by something unique.
    pub order: Vec<Sort>,
    /// Only rows that come after these values in `order`, one value per
    /// sort, compared in turn: keyset paging. None of them may be null.
    pub after: Option<Vec<Value>>,
    pub limit: Option<u64>,
}

impl Select {
    /// Every row of `table`, with these fields, in no particular order.
    pub fn new(table: &'static str, fields: Vec<Field>) -> Select {
        Select {
            table,
            fields,
            filter: Condition::True,
            order: Vec::new(),
            after: None,
            limit: None,
        }
    }

    /// Shorthand: these columns of `table`.
    pub fn columns(table: &'static str, columns: &[&'static str]) -> Select {
        Select::new(
            table,
            columns.iter().map(|c| Field::Value(col(c))).collect(),
        )
    }

    pub fn filter(mut self, filter: Condition) -> Select {
        self.filter = self.filter.and(filter);
        self
    }

    pub fn order(mut self, sort: Sort) -> Select {
        self.order.push(sort);
        self
    }

    pub fn after(mut self, values: Vec<Value>) -> Select {
        self.after = Some(values);
        self
    }

    pub fn limit(mut self, limit: u64) -> Select {
        self.limit = Some(limit);
        self
    }
}

/// How many rows of `table` meet `filter`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Count {
    pub table: &'static str,
    pub filter: Condition,
}

/// The rows of `table` that meet `filter`, in groups that share the value
/// of `by`. Each group gives one row: `by`, then each of `also` (read from
/// any row of the group, so each must have one value per group), then the
/// group's size, then the largest `max`. Groups come largest `max` first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub table: &'static str,
    pub by: Operand,
    pub also: Vec<Operand>,
    pub max: &'static str,
    pub filter: Condition,
}
