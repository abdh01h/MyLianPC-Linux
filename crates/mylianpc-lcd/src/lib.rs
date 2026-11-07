//! LCD themes, media decoding and JPEG/PNG encoding for the 480×480 panel.
//!
//! Ported from `MyLianPC.Lcd/{LcdRenderer,LcdMedia}.cs`. The service renders
//! frames headlessly and pushes them to the HydroShift II; the UI gets PNG
//! previews over IPC.

pub mod media;
pub mod render;
pub mod themes;

pub use media::LcdMedia;
pub use render::{encode_jpeg, encode_png, preview_transform, Canvas, FontCache, SIZE};
pub use themes::{format_metric, format_temp, clock_text, render, themes, LcdMetrics, LcdTheme};
