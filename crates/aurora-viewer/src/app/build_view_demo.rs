//! `AURORA_DEMO_BUILD_VIEW` scenarios (demo mode): the builder camera over
//! the demo work, panned with its own keys, then a prim taken and dragged
//! from it, so that the feature can be checked and captured without a grid.
//!
//! The prim is one the scenario rezzed itself: the demo plaza's own objects
//! belong to no one, and a grab of one of those is `Locked` (no move flag), so
//! the drag would show nothing.
//!
//! - `1`: the Focus tool opens at frame 560 and a 2 × 2 × 0.5 m prim is rezzed
//!   ten metres in front of the avatar; at 600 the eye stands over that prim,
//!   eight metres short of it, and pans forward from 610 to 630; the prim is
//!   taken at 660 (the Focus tool's press, after the depth under the cursor
//!   comes) and the cursor drags it right until 720, then releases it at 721.
//!   The prim's placement is logged before the drag and at 730.
//! - `ortho`: the same with the orthographic lens on from 620, so both lenses
//!   can be compared on the same scene.
//!
//! Capture around frame 700 (the drag) and 760 (after the release). The log
//! lines say what the camera stood over, whether the press took the prim, and
//! where the prim ended up.

use super::*;

/// The prim the scenario rezzed (0: none yet): the one it drags, so the drag
/// is read on a prim of ours whatever else stands in the view.
static TAKEN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Where the prim is now, for the log: the scenario must leave it moved.
fn placement(world: &crate::world::World, idx: usize) -> Option<Vec3> {
    crate::scene::Scene::object_transform(world, idx, std::time::Instant::now(), 0).map(|(p, _, _)| p)
}

impl App {
    /// The nearest root prim of our own (it can be moved) to `point`.
    fn rezzed_near(&self, point: Vec3) -> Option<usize> {
        let now = std::time::Instant::now();
        self.world
            .objects
            .iter()
            .filter(|(_, o)| !o.is_avatar() && o.parent_id == 0)
            .filter(|(_, o)| o.update_flags & crate::ui::context::flags::OBJECT_MOVE != 0)
            .filter_map(|(i, _)| {
                crate::scene::Scene::object_transform(&self.world, i, now, 0).map(|(p, _, _)| (i, p.distance_squared(point)))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    /// The frame's step of `AURORA_DEMO_BUILD_VIEW` (app.rs calls it once a
    /// frame while demo mode is on).
    pub(super) fn build_view_demo(&mut self) {
        let Ok(mode) = std::env::var("AURORA_DEMO_BUILD_VIEW") else {
            return;
        };
        if !self.demo || !self.in_world() {
            return;
        }
        let ortho = mode.trim().starts_with("ortho");
        match self.frame_count {
            545 => {
                self.panels.inventory = false;
                self.panels.chat = false;
                // a prim of our own to drag, on the ground ahead
                let p = self.world.agent.position;
                let (x, y) = (p.x + 10.0, p.y);
                let point = Vec3::new(x, y, crate::demo::floor_at(x, y) + 0.25);
                self.build.open_build(crate::build::Tool::Create);
                self.settings.build.new_prim_size = [2.0, 2.0, 0.5];
                self.build.mouse_down(
                    &mut self.world,
                    &mut self.settings.build,
                    Some(point),
                    None,
                    None,
                    self.cursor_pos,
                    crate::build::Mods::default(),
                );
                self.flush_build();
                log::info!("demo build view: rezzed a prim at {point:?}");
            }
            560 => {
                self.build.open_build(crate::build::Tool::Focus);
                log::info!("demo build view: Focus tool, selection {:?}", self.build.selection);
            }
            600 => {
                let Some(idx) = self.rezzed_near(self.world.agent.position + Vec3::new(10.0, 0.0, 0.0)) else {
                    log::warn!("demo build view: the rezzed prim is not in the world");
                    return;
                };
                TAKEN.store(idx + 1, std::sync::atomic::Ordering::Relaxed);
                let Some((point, _, _)) = crate::scene::Scene::object_transform(&self.world, idx, std::time::Instant::now(), 0) else {
                    return;
                };
                // the eye starts eight metres short of the prim, on the side
                // the shot already looks from, so the pan closes on it
                let short = point + Vec3::new(-8.0, 0.0, 0.0);
                self.camera.set_view(
                    short + Vec3::new(-4.0, -6.0, 3.0),
                    short,
                    &mut self.world.agent,
                    &self.settings.camera,
                );
                self.settings.build.build_view_ortho = false;
                self.build_view_set(true);
                log::info!(
                    "demo build view: {} over {:?} at {:?}",
                    ortho,
                    idx,
                    self.camera.build_view().map(|v| v.pose())
                );
            }
            // the orthographic lens on, before the pan ends (this arm comes
            // before the pan's, which would take the frame otherwise)
            620 if ortho => self.settings.build.build_view_ortho = true,
            610..=630 => {
                // the forward key, as `build_view_step` reads it, at 60 Hz
                if let Some(view) = self.camera.build_view_mut() {
                    view.step(
                        &crate::build::view::Move {
                            forward: 1.0,
                            ..Default::default()
                        },
                        1.0 / 60.0,
                    );
                }
            }
            655 => log::info!(
                "demo build view: after the pan {:?}, orthographic {}",
                self.camera.build_view().map(|v| v.pose()),
                self.camera.ortho_half_height().is_some()
            ),
            660 => {
                let taken = TAKEN.load(std::sync::atomic::Ordering::Relaxed);
                let Some(idx) = taken.checked_sub(1) else {
                    return;
                };
                let Some(point) = placement(&self.world, idx) else {
                    return;
                };
                let Some((x, y)) = self.build.cam.project_px(point) else {
                    log::warn!("demo build view: the prim is behind the camera");
                    return;
                };
                self.cursor_pos = (x, y);
                log::info!("demo build view: press at {x:.0},{y:.0} on {idx} at {point:?}");
                // the build tools' own press, as the mouse's: the Focus tool
                // takes the prim through `focus_tool_down`
                self.click(click_pick::Click::Build);
            }
            // the press asks the renderer for the depth under the cursor and
            // runs again when it comes, so the cursor waits until then
            661 => log::info!(
                "demo build view: after the press, waiting {} for the depth, grabbed {}",
                self.clicks.waiting(),
                self.build.grab.state.is_some()
            ),
            668 => log::info!(
                "demo build view: grabbed {}, selection {}",
                self.build.grab.state.is_some(),
                self.build.selection.len()
            ),
            670..=720 => {
                // the drag: the grab follows the cursor (build/mod.rs)
                self.cursor_pos.0 += 4.0;
            }
            721 => {
                self.on_left_release();
                log::info!("demo build view: released, grabbed {}", self.build.grab.state.is_some());
            }
            730 => {
                let taken = TAKEN.load(std::sync::atomic::Ordering::Relaxed);
                let now = taken.checked_sub(1).and_then(|idx| placement(&self.world, idx).map(|p| (idx, p)));
                log::info!("demo build view: after the drag {now:?}");
            }
            _ => {}
        }
    }
}
