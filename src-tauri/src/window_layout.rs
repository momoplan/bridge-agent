use anyhow::{Context, Result};
use std::fmt;
use tauri::{PhysicalPosition, PhysicalSize, WebviewWindow};

// Keep the window usable when space permits, while still allowing the responsive
// layout to reach its compact breakpoints on smaller displays.
const PREFERRED_MIN_INNER_WIDTH_LOGICAL: f64 = 560.0;
const PREFERRED_MIN_INNER_HEIGHT_LOGICAL: f64 = 420.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PhysicalRect {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WindowGeometry {
    outer_position: PhysicalPosition<i32>,
    outer_size: PhysicalSize<u32>,
    inner_size: PhysicalSize<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WindowLayoutPlan {
    inner_size: PhysicalSize<u32>,
    min_inner_size: PhysicalSize<u32>,
    outer_position: PhysicalPosition<i32>,
    resized: bool,
    repositioned: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowLayoutPolicy {
    Full,
    OversizedOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowLayoutOutcome {
    Applied {
        resized: bool,
        repositioned: bool,
        inner_size: PhysicalSize<u32>,
        outer_position: PhysicalPosition<i32>,
    },
    Unchanged,
    SkippedWindowMode,
    UnavailableMonitor,
}

impl fmt::Display for WindowLayoutOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Applied {
                resized,
                repositioned,
                inner_size,
                outer_position,
            } => write!(
                formatter,
                "applied resized={resized} repositioned={repositioned} inner={}x{} outer_position={},{}",
                inner_size.width, inner_size.height, outer_position.x, outer_position.y
            ),
            Self::Unchanged => formatter.write_str("unchanged"),
            Self::SkippedWindowMode => formatter.write_str("skipped_fullscreen_or_maximized"),
            Self::UnavailableMonitor => formatter.write_str("skipped_monitor_unavailable"),
        }
    }
}

pub(crate) fn fit_main_window_to_work_area(
    window: &WebviewWindow,
    policy: WindowLayoutPolicy,
) -> Result<WindowLayoutOutcome> {
    if window
        .is_fullscreen()
        .context("failed to read main window fullscreen state")?
        || window
            .is_maximized()
            .context("failed to read main window maximized state")?
    {
        return Ok(WindowLayoutOutcome::SkippedWindowMode);
    }

    let monitor = match window
        .current_monitor()
        .context("failed to resolve the main window monitor")?
    {
        Some(monitor) => Some(monitor),
        None => window
            .primary_monitor()
            .context("failed to resolve the primary monitor")?,
    };
    let Some(monitor) = monitor else {
        return Ok(WindowLayoutOutcome::UnavailableMonitor);
    };

    let work_area = monitor.work_area();
    if work_area.size.width == 0 || work_area.size.height == 0 {
        return Ok(WindowLayoutOutcome::UnavailableMonitor);
    }

    let geometry = WindowGeometry {
        outer_position: window
            .outer_position()
            .context("failed to read main window outer position")?,
        outer_size: window
            .outer_size()
            .context("failed to read main window outer size")?,
        inner_size: window
            .inner_size()
            .context("failed to read main window inner size")?,
    };
    let plan = plan_window_layout(
        PhysicalRect {
            x: work_area.position.x,
            y: work_area.position.y,
            width: work_area.size.width,
            height: work_area.size.height,
        },
        geometry,
        monitor.scale_factor(),
    );

    window
        .set_min_size(Some(plan.min_inner_size))
        .context("failed to set the adaptive main window minimum size")?;

    if policy == WindowLayoutPolicy::OversizedOnly && !plan.resized {
        return Ok(WindowLayoutOutcome::Unchanged);
    }
    if !plan.resized && !plan.repositioned {
        return Ok(WindowLayoutOutcome::Unchanged);
    }

    if plan.resized {
        window
            .set_size(plan.inner_size)
            .context("failed to fit the main window size to the monitor work area")?;
    }
    if plan.repositioned {
        window
            .set_position(plan.outer_position)
            .context("failed to fit the main window position to the monitor work area")?;
    }

    Ok(WindowLayoutOutcome::Applied {
        resized: plan.resized,
        repositioned: plan.repositioned,
        inner_size: plan.inner_size,
        outer_position: plan.outer_position,
    })
}

fn plan_window_layout(
    work_area: PhysicalRect,
    window: WindowGeometry,
    scale_factor: f64,
) -> WindowLayoutPlan {
    let frame_width = window
        .outer_size
        .width
        .saturating_sub(window.inner_size.width);
    let frame_height = window
        .outer_size
        .height
        .saturating_sub(window.inner_size.height);
    let max_inner_width = work_area.width.saturating_sub(frame_width).max(1);
    let max_inner_height = work_area.height.saturating_sub(frame_height).max(1);

    let normalized_scale = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    let preferred_min_width =
        logical_to_physical(PREFERRED_MIN_INNER_WIDTH_LOGICAL, normalized_scale)
            .min(max_inner_width);
    let preferred_min_height =
        logical_to_physical(PREFERRED_MIN_INNER_HEIGHT_LOGICAL, normalized_scale)
            .min(max_inner_height);

    let target_inner_size = PhysicalSize::new(
        window
            .inner_size
            .width
            .clamp(preferred_min_width, max_inner_width),
        window
            .inner_size
            .height
            .clamp(preferred_min_height, max_inner_height),
    );
    let target_outer_width = target_inner_size
        .width
        .saturating_add(frame_width)
        .min(work_area.width);
    let target_outer_height = target_inner_size
        .height
        .saturating_add(frame_height)
        .min(work_area.height);
    let target_outer_position = PhysicalPosition::new(
        clamp_axis(
            window.outer_position.x,
            work_area.x,
            work_area.width,
            target_outer_width,
        ),
        clamp_axis(
            window.outer_position.y,
            work_area.y,
            work_area.height,
            target_outer_height,
        ),
    );

    WindowLayoutPlan {
        inner_size: target_inner_size,
        min_inner_size: PhysicalSize::new(preferred_min_width, preferred_min_height),
        outer_position: target_outer_position,
        resized: target_inner_size != window.inner_size,
        repositioned: target_outer_position != window.outer_position,
    }
}

fn logical_to_physical(logical: f64, scale_factor: f64) -> u32 {
    (logical * scale_factor).round().clamp(1.0, u32::MAX as f64) as u32
}

fn clamp_axis(current: i32, start: i32, work_size: u32, outer_size: u32) -> i32 {
    let start = i64::from(start);
    let end = start + i64::from(work_size.saturating_sub(outer_size));
    i64::from(current)
        .clamp(start, end)
        .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry(
        x: i32,
        y: i32,
        outer_width: u32,
        outer_height: u32,
        inner_width: u32,
        inner_height: u32,
    ) -> WindowGeometry {
        WindowGeometry {
            outer_position: PhysicalPosition::new(x, y),
            outer_size: PhysicalSize::new(outer_width, outer_height),
            inner_size: PhysicalSize::new(inner_width, inner_height),
        }
    }

    #[test]
    fn shrinks_an_oversized_restored_window_inside_the_work_area() {
        let plan = plan_window_layout(
            PhysicalRect {
                x: 0,
                y: 24,
                width: 1024,
                height: 704,
            },
            geometry(80, 80, 1496, 1018, 1480, 980),
            1.0,
        );

        assert_eq!(plan.inner_size, PhysicalSize::new(1008, 666));
        assert_eq!(plan.outer_position, PhysicalPosition::new(0, 24));
        assert!(plan.resized);
        assert!(plan.repositioned);
    }

    #[test]
    fn keeps_a_fitting_window_and_position_unchanged() {
        let plan = plan_window_layout(
            PhysicalRect {
                x: 0,
                y: 24,
                width: 1920,
                height: 1056,
            },
            geometry(200, 100, 1216, 838, 1200, 800),
            1.0,
        );

        assert_eq!(plan.inner_size, PhysicalSize::new(1200, 800));
        assert_eq!(plan.outer_position, PhysicalPosition::new(200, 100));
        assert!(!plan.resized);
        assert!(!plan.repositioned);
    }

    #[test]
    fn moves_an_offscreen_window_on_a_negative_origin_monitor() {
        let plan = plan_window_layout(
            PhysicalRect {
                x: -1600,
                y: 30,
                width: 1600,
                height: 870,
            },
            geometry(-1900, -100, 1016, 738, 1000, 700),
            1.0,
        );

        assert_eq!(plan.outer_position, PhysicalPosition::new(-1600, 30));
        assert!(!plan.resized);
        assert!(plan.repositioned);
    }

    #[test]
    fn scales_the_preferred_minimum_for_high_dpi_displays() {
        let plan = plan_window_layout(
            PhysicalRect {
                x: 0,
                y: 0,
                width: 2560,
                height: 1440,
            },
            geometry(0, 0, 1016, 838, 1000, 800),
            1.5,
        );

        assert_eq!(plan.min_inner_size, PhysicalSize::new(840, 630));
        assert_eq!(plan.inner_size, PhysicalSize::new(1000, 800));
    }

    #[test]
    fn caps_the_minimum_when_the_work_area_is_smaller_than_preferred() {
        let plan = plan_window_layout(
            PhysicalRect {
                x: 0,
                y: 0,
                width: 800,
                height: 600,
            },
            geometry(0, 0, 1216, 938, 1200, 900),
            2.0,
        );

        assert_eq!(plan.min_inner_size, PhysicalSize::new(784, 562));
        assert_eq!(plan.inner_size, plan.min_inner_size);
        assert!(plan.resized);
    }

    #[test]
    fn invalid_scale_factor_falls_back_to_one() {
        let plan = plan_window_layout(
            PhysicalRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            geometry(0, 0, 816, 638, 800, 600),
            f64::NAN,
        );

        assert_eq!(plan.min_inner_size, PhysicalSize::new(560, 420));
    }

    #[test]
    fn extreme_virtual_desktop_coordinates_do_not_wrap() {
        assert_eq!(clamp_axis(i32::MAX, i32::MAX, u32::MAX, 1), i32::MAX);
    }
}
