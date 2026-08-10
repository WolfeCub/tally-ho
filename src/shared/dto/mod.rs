//! Wire types shared by server functions and the UI.
//!
//! Deliberately separate from [`crate::server::models`]: these compile for
//! wasm32, carry no toasty dependency, and are free to differ in shape from the
//! tables (e.g. [`ReceiptSummary`] folds line-item totals the DB can't sum).
//!
//! Shapes only. What they mean lives next door: [`crate::shared::problems`],
//! [`crate::shared::reconcile`], [`crate::shared::export`].

mod receipt;
mod statement;

pub use receipt::*;
pub use statement::*;
