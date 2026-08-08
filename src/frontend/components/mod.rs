//! The pieces the screens are built from — anything more than one screen needs,
//! or that would otherwise bury a screen's own markup.

mod form;
mod icons;
mod notice;
mod photo;
mod receipt_rows;
mod steps;

pub use form::{LabeledInput, field, form_element};
pub use icons::{CameraIcon, Spinner, Verdict};
pub use notice::{Notice, Tone};
pub use photo::ReceiptPhoto;
pub use receipt_rows::ReceiptRows;
pub use steps::StepBar;

/// Class strings that would otherwise drift apart as they get copied around.
/// Anything extra goes on the end: `format!("{INPUT} flex-1")`.
pub const INPUT: &str = "rounded-lg border border-edge bg-ink p-2";
pub const BUTTON: &str = "rounded-lg border border-edge bg-surface active:bg-edge";
/// The one thing a screen is really for. Everything else is a [`BUTTON`].
pub const PRIMARY: &str = "rounded-lg bg-paper font-medium text-ink active:opacity-80";
/// Big enough to hit with a thumb without aiming.
pub const TAP: &str = "min-h-11 px-4 py-3";
