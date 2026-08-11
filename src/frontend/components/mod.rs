//! The pieces the screens are built from — anything more than one screen needs,
//! or that would otherwise bury a screen's own markup.

mod form;
mod icons;
mod notice;
mod photo;
mod receipt_rows;
mod steps;

pub use form::{LabeledInput, confirm, field, form_element};
pub use icons::{CameraIcon, Spinner, Verdict};
pub use notice::{Notice, Tone, failed, loading};
pub use photo::ReceiptPhoto;
pub use receipt_rows::ReceiptRows;
pub use steps::StepBar;

/// Class strings that would otherwise drift apart as they get copied around.
/// Anything extra goes on the end: `format!("{BUTTON} w-full")`.
pub const INPUT: &str = "rounded-lg border border-edge bg-ink p-2";
/// Sized so a thumb hits it without aiming, and dimmed when disabled.
pub const BUTTON: &str = "min-h-11 rounded-lg border border-edge bg-surface px-4 py-3 \
                          active:bg-edge disabled:opacity-40";
/// The one thing a screen is really for. Everything else is a [`BUTTON`].
pub const PRIMARY: &str = "min-h-11 rounded-lg bg-paper px-4 py-3 font-medium text-ink \
                           active:opacity-80 disabled:opacity-40";
/// Not undoable, so it shouldn't look like the way out.
pub const DANGER: &str = "min-h-11 rounded-lg border border-danger px-4 py-3 text-danger";
/// An `<a>` that has to pass for a button.
pub const AS_BUTTON: &str = "flex items-center justify-center no-underline";
