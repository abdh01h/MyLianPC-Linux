//! Small shared helpers: messages, metric formatting and sparklines.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{DrawingArea, Label};

use mylianpc_core::color::RgbColor;
use mylianpc_hardware::sensors::Snapshot;

pub fn message_label() -> Label {
    let label = Label::new(None);
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_selectable(true);
    label.add_css_class("value-muted");
    label
}

pub fn show_result(label: &Label, result: Result<String, String>) {
    label.remove_css_class("error-text");
    match result {
        Ok(message) => label.set_text(&message),
        Err(message) => {
            label.add_css_class("error-text");
            label.set_text(&message);
        }
    }
}

pub fn show_error(label: &Label, message: impl Into<String>) {
    label.add_css_class("error-text");
    label.set_text(&message.into());
}

pub fn metric(snapshot: &Snapshot, key: &str, unit: &str, decimals: usize) -> String {
    match snapshot.values.get(key) {
        Some(value) => {
            if unit.is_empty() {
                format!("{value:.decimals$}")
            } else {
                format!("{value:.decimals$} {unit}")
            }
        }
        None => "—".to_string(),
    }
}

pub fn percent(snapshot: &Snapshot, key: &str) -> f64 {
    snapshot
        .values
        .get(key)
        .copied()
        .unwrap_or(0.0)
        .clamp(0.0, 100.0)
}

pub fn rate_text(bytes_per_second: f64) -> String {
    mylianpc_core::network::rate_text(bytes_per_second)
}

/// A small rolling line chart for the dashboard.
#[derive(Clone)]
pub struct Sparkline {
    points: Rc<RefCell<VecDeque<f64>>>,
    area: DrawingArea,
}

impl Sparkline {
    pub fn new(color: RgbColor, fixed_max: Option<f64>) -> (Self, DrawingArea) {
        let area = DrawingArea::new();
        area.set_content_height(72);
        area.set_hexpand(true);
        let points = Rc::new(RefCell::new(VecDeque::new()));
        let color_for_draw = color;
        let points_for_draw = Rc::clone(&points);
        area.set_draw_func(move |_, context, width, height| {
            let points = points_for_draw.borrow();
            if points.len() < 2 {
                return;
            }
            let max = fixed_max
                .unwrap_or_else(|| points.iter().copied().fold(1.0, f64::max))
                .max(1.0);
            let width = width as f64;
            let height = height as f64;
            let step = width / (points.len().saturating_sub(1).max(1)) as f64;
            context.set_line_width(2.0);
            context.set_source_rgba(
                color_for_draw.r as f64 / 255.0,
                color_for_draw.g as f64 / 255.0,
                color_for_draw.b as f64 / 255.0,
                0.95,
            );
            for (index, value) in points.iter().enumerate() {
                let x = index as f64 * step;
                let y = height - (value / max).clamp(0.0, 1.0) * (height - 4.0) - 2.0;
                if index == 0 {
                    context.move_to(x, y);
                } else {
                    context.line_to(x, y);
                }
            }
            let _ = context.stroke();
        });
        (Self { points, area: area.clone() }, area)
    }

    pub fn push(&self, value: f64) {
        let mut points = self.points.borrow_mut();
        points.push_back(value);
        while points.len() > 120 {
            points.pop_front();
        }
        drop(points);
        self.area.queue_draw();
    }
}
