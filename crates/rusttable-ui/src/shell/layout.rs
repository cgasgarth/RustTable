use iced::{Point, Size};

use super::model::WindowBounds;

#[derive(Debug, Clone, PartialEq)]
pub struct MonitorBounds {
    identity: String,
    position: Point,
    size: Size,
}

impl MonitorBounds {
    #[must_use]
    pub fn new(identity: String, position: Point, size: Size) -> Self {
        Self {
            identity,
            position,
            size,
        }
    }

    #[must_use]
    pub fn identity(&self) -> &str {
        &self.identity
    }

    #[must_use]
    pub const fn position(&self) -> Point {
        self.position
    }

    #[must_use]
    pub const fn size(&self) -> Size {
        self.size
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SavedWindowPlacement {
    monitor_identity: String,
    bounds: WindowBounds,
}

impl SavedWindowPlacement {
    #[must_use]
    pub fn new(monitor_identity: String, bounds: WindowBounds) -> Self {
        Self {
            monitor_identity,
            bounds,
        }
    }

    #[must_use]
    pub fn monitor_identity(&self) -> &str {
        &self.monitor_identity
    }

    #[must_use]
    pub const fn bounds(&self) -> WindowBounds {
        self.bounds
    }
}

#[must_use]
pub fn restore_placement(
    saved: &SavedWindowPlacement,
    monitors: &[MonitorBounds],
) -> SavedWindowPlacement {
    let monitor = monitors
        .iter()
        .find(|monitor| monitor.identity() == saved.monitor_identity())
        .or_else(|| monitors.first())
        .cloned()
        .unwrap_or_else(|| {
            MonitorBounds::new(
                String::from("primary"),
                Point::ORIGIN,
                Size::new(1_280.0, 800.0),
            )
        });
    let size = safe_size(saved.bounds().size(), monitor.size());
    let position = safe_position(saved.bounds().position(), size, &monitor);
    SavedWindowPlacement::new(
        monitor.identity().to_owned(),
        WindowBounds::new(position, size),
    )
}

fn safe_size(saved: Size, monitor: Size) -> Size {
    let width = saved.width.max(320.0).min(monitor.width.max(320.0));
    let height = saved.height.max(240.0).min(monitor.height.max(240.0));
    Size::new(width, height)
}

fn safe_position(position: Point, size: Size, monitor: &MonitorBounds) -> Point {
    let left = monitor.position().x;
    let top = monitor.position().y;
    let right = left + monitor.size().width - size.width;
    let bottom = top + monitor.size().height - size.height;
    Point::new(
        position.x.clamp(left, right.max(left)),
        position.y.clamp(top, bottom.max(top)),
    )
}

#[cfg(test)]
mod tests {
    use iced::{Point, Size};

    use super::{MonitorBounds, SavedWindowPlacement, restore_placement};
    use crate::shell::WindowBounds;

    #[test]
    fn known_monitor_keeps_identity_and_clamps_edges() {
        let saved = SavedWindowPlacement::new(
            String::from("secondary"),
            WindowBounds::new(Point::new(1_000.0, 1_000.0), Size::new(400.0, 300.0)),
        );
        let restored = restore_placement(
            &saved,
            &[MonitorBounds::new(
                String::from("secondary"),
                Point::new(100.0, 50.0),
                Size::new(800.0, 600.0),
            )],
        );

        assert_eq!(restored.monitor_identity(), "secondary");
        assert_eq!(restored.bounds().position(), Point::new(500.0, 350.0));
    }
}
