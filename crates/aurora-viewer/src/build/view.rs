//! Aurora's builder camera ("build view"): an eye over the work, tilted down,
//! that the movement keys fly while the build tools are open. It is the
//! camera of a modeller or a sandbox builder rather than of an avatar: the
//! avatar stays where it is, and the view pans across the ground instead.
//!
//! The Legacy Viewer has no such camera: its build tools keep the third
//! person camera behind the avatar, and `LLToolMgr::inBuildMode` only stops
//! the smoothing. This is Aurora's own addition, so it lives behind its own
//! setting (off by default, `BuildSettings::build_view`) and changes nothing
//! while it is off.
//!
//! The state here is pure: a focus point on the work, the heading the eye
//! stands at, the tilt below the horizon and the distance to the eye. The app
//! turns it into a camera pose (`Camera::start_build_view`), picking is left
//! alone, and `build/mod.rs` keeps the build tools as they are.

use glam::{Mat4, Vec3};

/// Degrees below the horizon: level would be 0, straight down 90. A view
/// never turns level (the work would be seen edge on) nor straight down (no
/// heading left to pan along).
pub const TILT_RANGE: std::ops::RangeInclusive<f32> = 15.0..=85.0;
/// Metres between the focus and the eye: close enough to place a prim, far
/// enough to see a linkset.
pub const DISTANCE_RANGE: std::ops::RangeInclusive<f32> = 3.0..=60.0;
/// Pan speed, metres a second at speed 1.
pub const SPEED_RANGE: std::ops::RangeInclusive<f32> = 2.0..=40.0;
/// Shift held: this much faster; Ctrl held: this much slower, as the photo
/// window's free camera does (`camera::free::FAST` / `SLOW`).
pub const FAST: f32 = 3.0;
pub const SLOW: f32 = 0.3;
/// A frame longer than this moves as if it lasted this long (moveFlycam's
/// "ridiculously big movements", as `camera::free::MAX_STEP_TIME`).
const MAX_STEP_TIME: f32 = 0.2;
/// One wheel click: the distance changes by this fraction.
const ZOOM_STEP: f32 = 0.12;
/// Pixels of a zoom drag that change the distance as one wheel click does.
const ZOOM_DRAG_PIXELS: f32 = 80.0;
/// Far plane of the orthographic lens: a region is 256 m across, so this
/// keeps four of them in depth without the perspective lens' infinite field.
const ORTHO_FAR: f32 = 1024.0;

fn clamp_tilt(degrees: f32) -> f32 {
    if degrees.is_finite() {
        degrees.clamp(*TILT_RANGE.start(), *TILT_RANGE.end())
    } else {
        40.0
    }
}

fn clamp_distance(metres: f32) -> f32 {
    if metres.is_finite() {
        metres.clamp(*DISTANCE_RANGE.start(), *DISTANCE_RANGE.end())
    } else {
        14.0
    }
}

fn clamp_speed(speed: f32) -> f32 {
    if speed.is_finite() {
        speed.clamp(*SPEED_RANGE.start(), *SPEED_RANGE.end())
    } else {
        12.0
    }
}

/// What the keys ask for this frame, each axis from -1 to 1 (keys at 1, a
/// joystick anywhere between) — the shape of `camera::free::FreeMove`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Move {
    pub forward: f32,
    pub left: f32,
    pub up: f32,
    pub fast: bool,
    pub slow: bool,
}

impl Move {
    pub fn is_still(&self) -> bool {
        self.forward == 0.0 && self.left == 0.0 && self.up == 0.0
    }
}

/// Where the builder camera is and what it looks at (render space).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BuildView {
    /// The point the view looks at, on the work.
    focus: Vec3,
    /// Heading the eye stands at, radians from +X, counter-clockwise.
    yaw: f32,
    /// Degrees below the horizon.
    tilt: f32,
    /// Metres between the focus and the eye.
    distance: f32,
    /// Metres a second the pan moves at speed 1.
    speed: f32,
    /// Whether the lens is orthographic rather than a field of view.
    ortho: bool,
}

impl Default for BuildView {
    fn default() -> Self {
        BuildView {
            focus: Vec3::ZERO,
            yaw: 0.0,
            tilt: 40.0,
            distance: 14.0,
            speed: 12.0,
            ortho: false,
        }
    }
}

impl BuildView {
    /// The view for a camera that already looks at `target` from `position`:
    /// it keeps the shot's heading, and takes the tilt and the distance asked
    /// for. What entering the mode does, so the view does not jump to the
    /// other side of the work.
    pub fn from_shot(position: Vec3, target: Vec3, tilt: f32, distance: f32, speed: f32) -> Self {
        let offset = position - target;
        let yaw = if offset.truncate().length_squared() > 1e-6 {
            offset.y.atan2(offset.x)
        } else {
            0.0
        };
        BuildView {
            focus: target,
            yaw,
            tilt: clamp_tilt(tilt),
            distance: clamp_distance(distance),
            speed: clamp_speed(speed),
            ortho: false,
        }
    }

    pub fn set_tilt(&mut self, degrees: f32) {
        self.tilt = clamp_tilt(degrees);
    }

    pub fn tilt(&self) -> f32 {
        self.tilt
    }

    pub fn distance(&self) -> f32 {
        self.distance
    }

    pub fn set_distance(&mut self, metres: f32) {
        self.distance = clamp_distance(metres);
    }

    pub fn set_speed(&mut self, speed: f32) {
        self.speed = clamp_speed(speed);
    }

    /// The lens: a field of view (false), or parallel rays (true).
    pub fn set_ortho(&mut self, on: bool) {
        self.ortho = on;
    }

    pub fn ortho(&self) -> bool {
        self.ortho
    }

    /// Half the world height the orthographic lens shows: the perspective
    /// lens' own picture of the work at the focus, so that switching keeps
    /// the framing, and the wheel still zooms.
    pub fn ortho_half_height(&self, fov_y: f32) -> f32 {
        (self.distance * (fov_y * 0.5).tan()).max(0.05)
    }

    /// The orthographic projection of a view of `aspect`, reversed-Z like the
    /// perspective lens (`Camera::proj`): 1 at the near plane, 0 at the far
    /// one, so the renderer's depth test keeps working unchanged.
    pub fn ortho_proj(&self, fov_y: f32, aspect: f32, near: f32) -> Mat4 {
        let half_h = self.ortho_half_height(fov_y);
        let half_w = half_h * aspect.max(0.1);
        glam::camera::rh::proj::directx::orthographic(
            -half_w,
            half_w,
            -half_h,
            half_h,
            // near and far swapped: the DirectX depth range reversed
            ORTHO_FAR,
            near.max(0.001),
        )
    }

    /// Focus to eye, level: the direction the eye stands in.
    fn flat(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        Vec3::new(cy, sy, 0.0)
    }

    /// Focus to eye, tilt included.
    fn offset(&self) -> Vec3 {
        let (st, ct) = self.tilt.to_radians().sin_cos();
        let flat = self.flat();
        Vec3::new(flat.x * ct, flat.y * ct, st) * self.distance
    }

    fn eye(&self) -> Vec3 {
        self.focus + self.offset()
    }

    /// Camera position and point looked at, as the camera wants them.
    pub fn pose(&self) -> (Vec3, Vec3) {
        (self.eye(), self.focus)
    }

    /// Pan the focus, raise or lower it, as the keys ask: level for the pan
    /// (the view keeps its tilt), in the view's own axes, never faster
    /// diagonally than straight. True when it moved.
    pub fn step(&mut self, m: &Move, dt: f32) -> bool {
        if m.is_still() {
            return false;
        }
        let dt = if dt.is_finite() { dt.clamp(0.0, MAX_STEP_TIME) } else { 0.0 };
        let gain = if m.fast {
            FAST
        } else if m.slow {
            SLOW
        } else {
            1.0
        };
        let flat = self.flat();
        let forward = -flat;
        // the view's own left: Vec3::Z.cross(forward), as `Camera::left_axis`
        let left = Vec3::new(flat.y, -flat.x, 0.0);
        let mut level = forward * m.forward + left * m.left;
        if level.length_squared() > 1.0 {
            level = level.normalize();
        }
        self.focus += level * self.speed * gain * dt;
        self.focus.z += m.up * self.speed * gain * dt;
        true
    }

    /// A drag of `dx`, `dy` pixels orbits the eye around its focus, as the
    /// modellers' and the city builders' cameras do: the heading turns with
    /// the drag across, the tilt follows it up and down, both kept in their
    /// range. The focus does not move, so the work stays framed.
    pub fn orbit(&mut self, dx: f32, dy: f32, sensitivity: f32) {
        if !dx.is_finite() || !dy.is_finite() {
            return;
        }
        // the camera's own rate (`camera::DRAG_RATE`), so a drag turns the
        // eye as fast as it turns the avatar's camera around the avatar
        let k = crate::camera::DRAG_RATE * sensitivity.max(0.0);
        let turn = std::f32::consts::PI;
        self.yaw = (self.yaw - dx * k + turn).rem_euclid(turn * 2.0) - turn;
        // down the screen raises the eye over the work, as the free
        // camera's drag looks down
        self.set_tilt(self.tilt + dy * k.to_degrees());
    }

    /// A drag of `dx`, `dy` pixels pans the focus across the view's own
    /// screen plane, so that the work follows the pointer: the world size of
    /// a pixel at the focus, from the lens and the window's height, the eye
    /// going the other way. The heading, the tilt and the distance stay.
    pub fn pan(&mut self, dx: f32, dy: f32, fov_y: f32, viewport_height: f32) {
        if !dx.is_finite() || !dy.is_finite() || !fov_y.is_finite() || !viewport_height.is_finite() || viewport_height <= 0.0 {
            return;
        }
        let per_pixel = 2.0 * self.distance * (fov_y * 0.5).tan().max(0.0) / viewport_height;
        let forward = (-self.offset()).normalize_or(Vec3::Z);
        // the view's own right, as `Camera::pan_offset`: forward x Z
        let right = forward.cross(Vec3::Z).normalize_or(Vec3::Y);
        let up = right.cross(forward).normalize_or(Vec3::Z);
        // the work goes right and down with the cursor, so the eye goes left
        // and up
        self.focus += (up * dy - right * dx) * per_pixel;
    }

    /// A zoom drag of `dy` pixels: up brings the eye closer, by the wheel's
    /// step every `ZOOM_DRAG_PIXELS`, the distance kept in its range.
    pub fn drag_zoom(&mut self, dy: f32) {
        if dy.is_finite() {
            self.zoom(-dy / ZOOM_DRAG_PIXELS);
        }
    }

    /// The wheel over the view: closer / farther, as a fraction of the
    /// distance (`clicks` positive brings the eye closer).
    pub fn zoom(&mut self, clicks: f32) {
        if clicks != 0.0 && clicks.is_finite() {
            self.set_distance(self.distance * (1.0 - ZOOM_STEP * clicks));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A shot looking at the origin from the +X side, 5 m above it.
    fn view() -> BuildView {
        BuildView::from_shot(Vec3::new(10.0, 0.0, 5.0), Vec3::ZERO, 40.0, 14.0, 12.0)
    }

    #[test]
    fn the_eye_stands_over_the_focus_at_the_tilt_asked() {
        let mut v = view();
        v.set_distance(10.0);
        v.set_tilt(45.0);
        let (eye, focus) = v.pose();
        assert_eq!(focus, Vec3::ZERO);
        assert!((eye.z - 10.0 * 45f32.to_radians().sin()).abs() < 1e-5);
        assert!(eye.x > focus.x, "the eye stands on the +X side of the focus");
        assert!((eye - focus).length() - 10.0 < 1e-4);
    }

    #[test]
    fn the_tilt_stays_in_range_and_a_broken_one_does_not_stick() {
        let mut v = BuildView::default();
        v.set_tilt(180.0);
        assert_eq!(v.tilt, *TILT_RANGE.end());
        v.set_tilt(-4.0);
        assert_eq!(v.tilt, *TILT_RANGE.start());
        v.set_tilt(f32::NAN);
        assert!(TILT_RANGE.contains(&v.tilt));
        // the defaults of `BuildSettings` are in range too
        let settings = crate::build::BuildSettings::default();
        assert!(TILT_RANGE.contains(&settings.build_view_tilt));
        assert!(DISTANCE_RANGE.contains(&settings.build_view_distance));
        assert!(SPEED_RANGE.contains(&settings.build_view_speed));
    }

    #[test]
    fn entering_from_a_shot_keeps_the_side_it_looked_from() {
        let v = view();
        assert!(v.yaw.abs() < 1e-6, "yaw {} is not the shot's heading", v.yaw);
        assert_eq!(v.tilt, 40.0);
        assert_eq!(v.distance, 14.0);
    }

    #[test]
    fn forward_pans_away_from_the_eye_and_the_pan_stays_level() {
        let mut v = view();
        let before = v.focus;
        let eye = v.pose().0;
        let m = Move {
            forward: 1.0,
            ..Move::default()
        };
        // a full second is capped: the step lasts MAX_STEP_TIME
        assert!(v.step(&m, 1.0));
        let delta = v.focus - before;
        assert!(delta.x < 0.0, "forward goes away from the eye at yaw 0");
        assert_eq!(delta.z, 0.0, "the pan is level");
        assert!((delta.length() - v.speed * MAX_STEP_TIME).abs() < 1e-4);
        assert!((v.pose().0 - eye).truncate().length() > 0.0, "the eye followed");
        assert!((v.pose().0 - v.focus).length() - 14.0 < 1e-4);
    }

    #[test]
    fn left_pans_to_the_views_left_and_a_diagonal_is_no_faster() {
        let mut v = view();
        let before = v.focus;
        let m = Move {
            left: 1.0,
            ..Move::default()
        };
        v.step(&m, 1.0);
        assert!(v.focus.y < before.y, "facing -X, the view's left is -Y");
        let start = v.focus;
        let m = Move {
            forward: 1.0,
            left: 1.0,
            ..Move::default()
        };
        v.step(&m, MAX_STEP_TIME);
        let diagonal = (v.focus - start).length();
        assert!(
            (diagonal - v.speed * MAX_STEP_TIME).abs() < 1e-3,
            "diagonal {diagonal} beats straight"
        );
    }

    #[test]
    fn up_raises_the_focus_and_long_frames_are_capped() {
        let mut v = BuildView::default();
        let m = Move {
            up: 1.0,
            ..Move::default()
        };
        v.step(&m, 10.0);
        assert!((v.focus.z - v.speed * MAX_STEP_TIME).abs() < 1e-5);
        assert!(!v.step(&Move::default(), 1.0));
    }

    #[test]
    fn fast_and_slow_modifiers_scale_the_pan() {
        let mut v = BuildView::default();
        let before = v.focus;
        v.step(
            &Move {
                forward: 1.0,
                fast: true,
                ..Move::default()
            },
            1.0,
        );
        let fast = (v.focus - before).length();
        let before = v.focus;
        v.step(
            &Move {
                forward: 1.0,
                slow: true,
                ..Move::default()
            },
            1.0,
        );
        let slow = (v.focus - before).length();
        assert!(fast > slow * 4.0, "fast {fast} is not clearly faster than slow {slow}");
    }

    #[test]
    fn the_orthographic_lens_is_reversed_and_keeps_the_framing() {
        let mut v = BuildView::default();
        assert!(!v.ortho(), "the lens is a field of view until asked");
        v.set_ortho(true);
        assert!(v.ortho());
        let (fov_y, aspect, near) = (0.8f32, 16.0 / 9.0, 0.1f32);
        let p = v.ortho_proj(fov_y, aspect, near);
        // view space: the camera looks down -Z
        let depth = |z: f32| {
            let c = p * glam::Vec4::new(0.0, 0.0, -z, 1.0);
            c.z / c.w
        };
        assert!((depth(near) - 1.0).abs() < 1e-4, "near {} is not 1", depth(near));
        assert!(depth(ORTHO_FAR).abs() < 1e-4, "far {} is not 0", depth(ORTHO_FAR));
        assert!(depth(near) > depth(ORTHO_FAR / 2.0), "depth is not reversed");
        // the same picture as the perspective lens at the focus
        assert!((v.ortho_half_height(fov_y) - 14.0 * (fov_y * 0.5).tan()).abs() < 1e-5);
        let corner = p * glam::Vec4::new(v.ortho_half_height(fov_y) * aspect, 0.0, -1.0, 1.0);
        assert!((corner.x / corner.w - 1.0).abs() < 1e-4, "the width does not follow the aspect");
    }

    #[test]
    fn a_drag_orbits_the_eye_around_its_focus() {
        let mut v = view();
        let (focus, distance) = (v.focus, v.distance);
        let eye = v.pose().0;
        v.orbit(100.0, 0.0, 1.0);
        // the heading turns with the drag, the focus and the distance stay
        assert!(
            (v.yaw + 100.0 * crate::camera::DRAG_RATE).abs() < 1e-5,
            "yaw {} did not turn",
            v.yaw
        );
        assert_eq!(v.focus, focus);
        assert_eq!(v.distance, distance);
        let after = v.pose().0;
        assert!((after - eye).truncate().length() > 1.0, "the eye stayed put");
        assert!(((after - v.focus).length() - distance).abs() < 1e-4, "the eye left its circle");
        // down the screen raises the eye over the work, a pixel for a pixel
        let tilt = v.tilt;
        v.orbit(0.0, 50.0, 1.0);
        assert!(
            (v.tilt - (tilt + 50.0 * crate::camera::DRAG_RATE.to_degrees())).abs() < 1e-4,
            "tilt {} did not follow the drag",
            v.tilt
        );
        // the heading comes back in range, a whole turn being the same shot
        v.orbit(100_000.0, 0.0, 1.0);
        assert!(v.yaw.is_finite() && v.yaw.abs() <= std::f32::consts::PI + 1e-4);
    }

    #[test]
    fn a_pan_drag_moves_the_work_with_the_pointer() {
        let mut v = view();
        let (yaw, tilt, distance) = (v.yaw, v.tilt, v.distance);
        let focus = v.focus;
        // 0.8 rad over 900 px at 14 m: about 13 mm a pixel
        let per_pixel = 2.0 * 14.0 * (0.8f32 * 0.5).tan() / 900.0;
        v.pan(100.0, 0.0, 0.8, 900.0);
        let right = (-v.flat()).cross(Vec3::Z).normalize_or(Vec3::Y);
        let moved = v.focus - focus;
        assert!(
            (moved + right * 100.0 * per_pixel).length() < 1e-4,
            "the eye did not go left with the pointer: {moved:?}"
        );
        assert_eq!((v.yaw, v.tilt, v.distance), (yaw, tilt, distance));
        // down the screen raises the eye, so the work goes down with the
        // cursor
        let before = v.focus;
        v.pan(0.0, 100.0, 0.8, 900.0);
        assert!(v.focus.z > before.z, "the eye did not rise: {:?}", v.focus - before);
        // a broken drag or a window with no height moves nothing
        let (focus, yaw) = (v.focus, v.yaw);
        v.pan(f32::NAN, 0.0, 0.8, 900.0);
        v.pan(10.0, 10.0, 0.8, 0.0);
        assert_eq!((v.focus, v.yaw), (focus, yaw));
    }

    #[test]
    fn a_zoom_drag_changes_the_distance_as_the_wheel_does() {
        let mut v = view();
        v.drag_zoom(-ZOOM_DRAG_PIXELS);
        assert!((v.distance - 14.0 * (1.0 - ZOOM_STEP)).abs() < 1e-4, "{}", v.distance);
        // dragging down moves the eye away
        let before = v.distance;
        v.drag_zoom(ZOOM_DRAG_PIXELS * 2.0);
        assert!(v.distance > before);
        for _ in 0..50 {
            v.drag_zoom(-ZOOM_DRAG_PIXELS);
        }
        assert_eq!(v.distance, *DISTANCE_RANGE.start());
        v.drag_zoom(f32::NAN);
        assert_eq!(v.distance, *DISTANCE_RANGE.start());
    }

    #[test]
    fn the_orbit_keeps_the_tilt_in_range_and_ignores_a_broken_drag() {
        let mut v = view();
        v.orbit(0.0, 100_000.0, 1.0);
        assert_eq!(v.tilt, *TILT_RANGE.end());
        v.orbit(0.0, -100_000.0, 1.0);
        assert_eq!(v.tilt, *TILT_RANGE.start());
        let (yaw, tilt) = (v.yaw, v.tilt);
        v.orbit(f32::NAN, f32::INFINITY, 1.0);
        assert_eq!((v.yaw, v.tilt), (yaw, tilt));
    }

    #[test]
    fn the_wheel_keeps_the_distance_in_range() {
        let mut v = BuildView::default();
        for _ in 0..50 {
            v.zoom(1.0);
        }
        assert_eq!(v.distance(), *DISTANCE_RANGE.start());
        for _ in 0..50 {
            v.zoom(-1.0);
        }
        assert_eq!(v.distance(), *DISTANCE_RANGE.end());
    }
}
