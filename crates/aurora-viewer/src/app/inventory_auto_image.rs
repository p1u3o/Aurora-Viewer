//! Automatic picture of an attachable inventory item (Aurora addition).
//!
//! The Legacy Viewer's Change item thumbnail takes a snapshot of whatever
//! is on screen. Here the item is worn for the picture, the isolation shows
//! that attachment alone (its wearer is only unseen, so a rigged garment
//! still has a posed skeleton and a FULL cull state), and the square photo
//! becomes the item's picture through the ordinary thumbnail upload. The
//! camera is levelled from the attachment's own down axis and faces the rig
//! joint it hangs from: a non-rigged object carried at Spine has the point's
//! rotation applied to it, a rigged one is posed from the skeleton instead.
//!
//! Nothing here changes the grid: the wear is the same as double-clicking
//! the item, and the previous outfit is put back when it was not already
//! worn (the window asks for consent first).

use super::App;
use super::snapshot::Purpose;
use crate::camera::FixedView;
use crate::scene::textures::PreviewSource;
use crate::snapshot::isolate::{self, Isolation, Mode};
use crate::snapshot::portrait;
use crate::ui::widgets::{Floater, flat_button, primary_button_sized};
use crate::wearable_editor::preview::{self as bake_preview, Reply as BakeReply};
use aurora_assets::wearable::Wearable;
use aurora_render::Renderer;
use egui::Vec2;
use glam::{Quat, Vec3};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// How long to wait for the simulator to build the worn object.
const DRESS_WAIT: Duration = Duration::from_secs(15);
/// The camera's margin around the attachment's bounds.
const FRAMING_MARGIN: f32 = 1.5;
/// The preview is drawn again this often while it is open.
const PREVIEW_EVERY: Duration = Duration::from_millis(500);

/// Only an object item that the simulator can attach. Wearables and
/// textures are not objects the viewer can photograph this way.
fn eligible(item: &aurora_net::inventory::InvItem) -> bool {
    matches!(item.asset_type, 5 | 6 | 13)
}

/// A system wearable photographed from locally composed bakes on a white
/// base: the avatar is not undressed and nothing is sent to the grid.
struct WearablePhoto {
    value: Wearable,
    sources: HashMap<Uuid, PreviewSource>,
    rx: Option<crossbeam_channel::Receiver<BakeReply>>,
    prepared: HashMap<usize, (Uuid, u32)>,
    done: bool,
}

/// World "up" for the photo. A non-rigged attachment carries the point's
/// rotation in its world transform (`w_r * f_rot * r`), so its local +Z is
/// `Q * Z`; a rigged mesh is posed by the skeleton and never sees that
/// rotation. A HUD has no world down and keeps the ordinary view.
fn subject_up(rigged: bool, object_rot: Quat, wearer_rot: Option<Quat>) -> Vec3 {
    if rigged {
        wearer_rot.map(|q| q * Vec3::Z).unwrap_or(Vec3::Z)
    } else {
        object_rot * Vec3::Z
    }
}

/// World direction the worn item was authored facing. Rigged content is
/// authored facing the avatar's +Y, but the rig joint the item hangs from can
/// turn independently of the avatar's root — a turned head, a pointed foot —
/// and `attach_frame` already carries that joint's posed rotation. Reading +Y
/// in that frame gives a better front than the root's facing; without a known
/// point (or for a non-rigged prim, which carries the point's rotation itself)
/// the avatar's own forward is kept. The component along world Z is dropped:
/// the camera stays level even when the joint points up or down.
fn subject_front(rigged: bool, wearer_rot: Option<Quat>, point_rot: Option<Quat>) -> Vec3 {
    let wearer_rot = wearer_rot.unwrap_or(Quat::IDENTITY);
    let front = if rigged {
        wearer_rot * point_rot.unwrap_or(Quat::IDENTITY) * Vec3::Y
    } else {
        wearer_rot * Vec3::Y
    };
    Vec3::new(front.x, front.y, 0.0)
}

/// Whether `idx` is the attachment root or one of its linked children.
fn under_root(world: &crate::world::World, idx: usize, root: usize) -> bool {
    let mut idx = idx;
    for _ in 0..64 {
        if idx == root {
            return true;
        }
        let Some(object) = world.objects.get(idx) else {
            return false;
        };
        let Some(parent) = world.objects.parent_of(object) else {
            return false;
        };
        idx = parent;
    }
    false
}

impl App {
    /// Bounds of the whole worn item, not just its root prim: a garment is
    /// often a linkset whose visible panels are children of a scripted or
    /// invisible root. Centre-cropping the root alone leaves the picture
    /// badly off centre and makes rotation swing around the wrong point.
    fn subject_bounds(&self, root: usize, rigged: bool) -> Option<(Vec3, f32)> {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for (idx, _) in self.world.objects.iter() {
            if !under_root(&self.world, idx, root) {
                continue;
            }
            let g = self.scene.gpu.get(idx).filter(|g| !g.faces.is_empty());
            let bounds = if rigged {
                self.scene.posed_object_bounds(&self.world, idx)
            } else {
                None
            }
            .or_else(|| g.map(|g| (g.center, g.radius)));
            if let Some((center, radius)) = bounds {
                let span = Vec3::splat(radius);
                min = min.min(center - span);
                max = max.max(center + span);
            }
        }
        if min.x.is_finite() {
            Some(((min + max) * 0.5, (max - min).length() * 0.5))
        } else {
            None
        }
    }
}

/// The photo awaiting "Use".
struct Shot {
    size: (u32, u32),
    pixels: Vec<u8>,
    texture: egui::TextureHandle,
}

pub(super) struct Session {
    item: Uuid,
    /// The worn attachment's root prim, once it arrives.
    attachment: Option<Uuid>,
    /// Set for a system wearable: no object is worn for it.
    wearable: Option<WearablePhoto>,
    /// The local T-pose held for the photo, on this screen only.
    pose: Option<Arc<crate::ik_edit::poser::SubjectPose>>,
    /// The item was already worn: do not take it off at the end.
    was_worn: bool,
    camera: crate::camera::Camera,
    fixed: Option<FixedView>,
    isolation: Isolation,
    wait: Option<Instant>,
    /// The user's framing, in degrees around the wearer and as a distance
    /// multiplier on the automatic fit.
    yaw: f32,
    zoom: f32,
    preview_at: Option<Instant>,
    preview: Option<egui::TextureHandle>,
    shot: Option<Shot>,
    shot_wanted: bool,
    shooting: bool,
    error: String,
    saved: bool,
    /// The camera, isolation and the temporary wear are back as they were;
    /// the window stays open only to show the result or the error.
    restored: bool,
    /// The window's first step: the item has not been worn yet.
    confirming: bool,
    open: bool,
}

impl Session {
    /// A shot of this session is still waiting for the renderer.
    pub(super) fn is_shooting(&self) -> bool {
        self.shooting && !self.restored
    }
}

impl App {
    /// The right-click command and the picture editor's Auto button.
    pub(super) fn open_inventory_auto_image(&mut self, item: Uuid) {
        if self.inventory_auto_image.is_some() {
            self.close_inventory_auto_image();
        }
        let Some(it) = self.world.inventory.items.get(&item) else {
            return;
        };
        if !eligible(it) {
            return;
        }
        // An item already on the avatar is photographed as it is: no second
        // wear, and nothing to take off at the end. A system wearable never
        // needs wearing: its textures are baked onto a white avatar locally.
        let was_worn = self.world.worn_attachment_items().contains(&item);
        let wearable = it.asset_type == 5 || it.asset_type == 13;
        let record = it.clone();
        log::info!("inventory auto image: opened for {item}, worn {was_worn}, wearable {wearable}");
        self.inventory_auto_image = Some(Session {
            item,
            attachment: None,
            wearable: None,
            pose: None,
            was_worn,
            camera: self.camera.clone(),
            fixed: self.camera.fixed_view,
            isolation: self.world.photo_isolation.clone(),
            wait: None,
            yaw: 0.0,
            zoom: 1.0,
            preview_at: None,
            preview: None,
            shot: None,
            shot_wanted: false,
            shooting: false,
            error: String::new(),
            saved: false,
            restored: false,
            confirming: !was_worn && !wearable,
            open: true,
        });
        crate::ui::widgets::open_floater(&self.egui_ctx, WINDOW, &mut true);
        if was_worn {
            self.begin_inventory_auto_image();
        } else if wearable {
            self.start_wearable_photo(record);
        }
    }

    pub(super) fn inventory_auto_image_active(&self) -> bool {
        self.inventory_auto_image
            .as_ref()
            .is_some_and(|s| !s.confirming && !s.saved && !s.restored)
    }

    /// The isolation stays open for the picture with the photo mode closed.
    pub(super) fn inventory_auto_image_isolates(&self) -> bool {
        self.inventory_auto_image_active()
    }

    pub(super) fn begin_inventory_auto_image(&mut self) {
        let Some(item) = self.inventory_auto_image.as_ref().map(|s| s.item) else {
            return;
        };
        let was_worn = self.inventory_auto_image.as_ref().is_some_and(|s| s.was_worn);
        if let Some(s) = &mut self.inventory_auto_image {
            s.confirming = false;
            s.wait = Some(Instant::now());
        }
        if self.demo {
            // The offline demo has no simulator to rez the item. The
            // two-prim backpack stands in for an attachment; its point (9)
            // has a tilt, which is what the levelling has to correct.
            for event in crate::demo::worn::events() {
                self.world.apply(event);
            }
            let root = crate::demo::worn::root().full_id;
            if let Some(s) = &mut self.inventory_auto_image {
                s.attachment = Some(root);
            }
        } else if !was_worn {
            self.apply_appearance_action(crate::world::appearance::Action::Add(item));
        }
    }

    /// A local T-pose for the picture, after the ordinary pose layers have
    /// been built (as the outfit portrait does). Nothing is sent to the grid.
    pub(super) fn inventory_auto_image_pose(&mut self) {
        let Some(s) = self.inventory_auto_image.as_ref() else {
            return;
        };
        if s.confirming || s.saved || s.restored {
            return;
        }
        let id = self.world.agent_id;
        if self.scene.poser.contains_key(&id) {
            // the user's own pose stays their own
            return;
        }
        if s.pose.is_none()
            && let Some(pose) = crate::ik_edit::poser::standing_pose(&self.world, id, 0.0)
            && let Some(s) = &mut self.inventory_auto_image
        {
            s.pose = Some(pose);
        }
        if let Some(pose) = self.inventory_auto_image.as_ref().and_then(|s| s.pose.clone()) {
            self.scene.poser.insert(id, pose);
        }
    }

    /// The camera, the isolation and the temporary wear back as they were.
    /// The window stays for the result, so this is not the session's end.
    fn end_inventory_auto_image_photo(&mut self) {
        let Some(s) = &mut self.inventory_auto_image else {
            return;
        };
        if s.restored {
            return;
        }
        s.restored = true;
        let pose = s.pose.take();
        s.shot_wanted = false;
        s.shooting = false;
        s.wait = None;
        let (camera, fixed, isolation) = (s.camera.clone(), s.fixed, s.isolation.clone());
        let (item, was_worn, confirming, had_wearable) = (s.item, s.was_worn, s.confirming, s.wearable.is_some());
        if let Some(pose) = pose {
            let id = self.world.agent_id;
            if self.scene.poser.get(&id).is_some_and(|held| Arc::ptr_eq(held, &pose)) {
                self.scene.poser.remove(&id);
            }
        }
        if had_wearable {
            self.world.preview_bakes(HashMap::new());
        }
        self.camera = camera;
        self.camera.fixed_view = fixed;
        // Bumped past the current version: the scene re-flags every object.
        let version = self.world.photo_isolation.version;
        self.world.photo_isolation = isolation;
        self.world.photo_isolation.version = version.wrapping_add(1);
        if !was_worn && !confirming && !self.demo {
            self.apply_appearance_action(crate::world::appearance::Action::Remove(item));
        }
        log::info!("inventory auto image: camera, isolation and outfit restored");
    }

    pub(super) fn close_inventory_auto_image(&mut self) {
        self.end_inventory_auto_image_photo();
        if self.inventory_auto_image.take().is_some() {
            log::info!("inventory auto image: closed");
        }
    }

    /// The picture's pixels, once the renderer has drawn them.
    pub(super) fn inventory_auto_image_drawn(&mut self, preview: bool, size: (u32, u32), mut pixels: Vec<u8>, cutout: bool) {
        let Some(s) = &mut self.inventory_auto_image else {
            return;
        };
        if preview {
            let image = egui::ColorImage::from_rgba_unmultiplied([size.0 as usize, size.1 as usize], &pixels);
            s.preview = Some(
                self.egui_ctx
                    .load_texture("inventory_auto_image_preview", image, egui::TextureOptions::LINEAR),
            );
            return;
        }
        if cutout {
            portrait::flatten(
                &mut pixels,
                portrait::backdrop_rgb(portrait::Backdrop::Grey).unwrap_or([64, 64, 64]),
            );
        }
        let texture = self.egui_ctx.load_texture(
            "inventory_auto_image_shot",
            egui::ColorImage::from_rgba_unmultiplied([size.0 as usize, size.1 as usize], &pixels),
            egui::TextureOptions::LINEAR,
        );
        s.shot = Some(Shot { size, pixels, texture });
        s.shooting = false;
    }

    /// The upload has answered: the window says whether it worked. A failed
    /// picture is not left with the item worn and the scene isolated.
    pub(super) fn inventory_auto_image_upload_done(&mut self, error: Option<String>) {
        let error = error.unwrap_or_default();
        if let Some(s) = &mut self.inventory_auto_image {
            s.saved = error.is_empty();
            s.error = error;
        } else {
            return;
        }
        self.end_inventory_auto_image_photo();
    }

    /// The renderer could not draw the photo: say so here, and put the scene
    /// and the outfit back instead of waiting for a shot that never comes.
    pub(super) fn inventory_auto_image_photo_failed(&mut self, error: &str) {
        if let Some(s) = &mut self.inventory_auto_image
            && s.shooting
        {
            s.error = error.to_owned();
        } else {
            return;
        }
        self.end_inventory_auto_image_photo();
    }

    /// Fetch the LLWearable text asset. In demo mode we make one locally, as
    /// the wearable editor does; on the grid the region's ViewerAsset cap
    /// answers with the ordinary permission check.
    fn start_wearable_photo(&mut self, record: aurora_net::inventory::InvItem) {
        let item = record.id;
        if self.demo {
            let kind = 4; // a system shirt, as the demo wearable editor uses
            let bytes = crate::world::inventory::wearable::create(kind, "Demo wearable", record.owner).unwrap_or_default();
            self.wearable_loaded(item, Ok(bytes));
        } else {
            self.send(aurora_net::NetCommand::PreviewInventoryItem(record));
        }
    }

    /// Whether this preview reply belongs to a wearable picture in progress.
    pub(super) fn wearable_photo_pending(&self, item: Uuid) -> bool {
        self.inventory_auto_image
            .as_ref()
            .is_some_and(|s| s.item == item && !s.restored && s.wearable.is_none())
    }

    /// The wearable's text and its texture slots are here: prepare the white
    /// bakes once all referenced textures are decoded.
    pub(super) fn wearable_loaded(&mut self, item: Uuid, result: Result<Vec<u8>, String>) {
        if self
            .inventory_auto_image
            .as_ref()
            .is_none_or(|s| s.item != item || s.wearable.is_none() && s.confirming)
        {
            return;
        }
        let bytes = match result {
            Ok(bytes) => bytes,
            Err(error) => {
                if let Some(s) = &mut self.inventory_auto_image {
                    s.error = error;
                }
                self.end_inventory_auto_image_photo();
                return;
            }
        };
        let value = match Wearable::parse(&bytes) {
            Ok(value) => value,
            Err(error) => {
                if let Some(s) = &mut self.inventory_auto_image {
                    s.error = format!("invalid wearable: {error}");
                }
                self.end_inventory_auto_image_photo();
                return;
            }
        };
        let affected = crate::wearable_editor::affected_bakes(value.kind);
        let ids: Vec<Uuid> = value
            .textures
            .iter()
            .filter_map(|(slot, id)| {
                crate::wearable_editor::slot_bake(*slot)
                    .filter(|te| affected.contains(te))
                    .map(|_| *id)
            })
            .filter(|id| {
                !id.is_nil()
                    && *id != aurora_prim::te::BLANK_TEXTURE
                    && *id != crate::scene::avatar::IMG_DEFAULT_AVATAR
                    && *id != crate::scene::avatar::IMG_INVISIBLE
            })
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        let mut sources = HashMap::new();
        for id in ids {
            self.scene.textures.note_usage_boosted(&id, f32::MAX);
            sources.insert(id, self.scene.textures.preview_source(&id));
        }
        log::info!(
            "inventory auto image: wearable {} kind {}, {} texture source(s)",
            item,
            value.kind,
            sources.len()
        );
        if let Some(s) = &mut self.inventory_auto_image {
            s.wearable = Some(WearablePhoto {
                value,
                sources,
                rx: None,
                prepared: HashMap::new(),
                done: false,
            });
        }
    }

    /// Drive the bake job and apply its textures to our own avatar.
    fn wearable_frame(&mut self, renderer: &mut Renderer) {
        let (rx, ready, kind, value, sources) = {
            let Some(s) = self.inventory_auto_image.as_ref() else {
                return;
            };
            let Some(w) = s.wearable.as_ref() else {
                return;
            };
            (
                w.rx.clone(),
                w.sources.iter().all(|(id, source)| source.bake_ready(*id)),
                w.value.kind,
                w.value.clone(),
                w.sources.clone(),
            )
        };
        if rx.is_none() {
            if !ready {
                return;
            }
            let Some(pool) = self.scene.textures.staging.clone() else {
                log::warn!("inventory auto image: no staging pool for the wearable bake");
                return;
            };
            let lib = self.world.avatar_lib.clone();
            let base = self.world.server_visual_params().to_vec();
            let (tx, rx) = crossbeam_channel::unbounded();
            let token = Arc::new(AtomicU64::new(1));
            self.scene.jobs.execute(move || {
                match bake_preview::prepare(1, &token, &pool, &lib, &base, kind, vec![value], sources, &tx, None, true) {
                    Ok(()) => {
                        let _ = tx.send(BakeReply::Done(1));
                    }
                    Err(error) => {
                        log::warn!("inventory auto image: wearable bake failed: {error}");
                        let _ = tx.send(BakeReply::Failed(1, error));
                    }
                }
            });
            if let Some(s) = &mut self.inventory_auto_image
                && let Some(w) = &mut s.wearable
            {
                w.rx = Some(rx);
            }
            return;
        }
        let mut done = false;
        let mut failed = None;
        while let Ok(reply) = rx.as_ref().expect("bake receiver").try_recv() {
            match reply {
                BakeReply::Texture(upload) => {
                    let Some(slot) = renderer.textures.create_placeholder() else {
                        failed = Some("out of texture slots".to_owned());
                        break;
                    };
                    if renderer.replace_texture_staged(slot, &upload.texture.staged) {
                        self.scene
                            .textures
                            .register_local_image(upload.id, slot, upload.texture.alpha, upload.texture.has_alpha);
                        if let Some(s) = &mut self.inventory_auto_image
                            && let Some(w) = &mut s.wearable
                        {
                            w.prepared.insert(upload.te, (upload.id, slot));
                        }
                    } else {
                        renderer.free_texture(slot);
                        failed = Some("could not upload a bake".to_owned());
                        break;
                    }
                }
                BakeReply::Done(_) => done = true,
                BakeReply::Failed(_, error) => failed = Some(error),
            }
        }
        if let Some(error) = failed {
            if let Some(s) = &mut self.inventory_auto_image {
                s.error = error;
            }
            self.end_inventory_auto_image_photo();
            return;
        }
        if !done {
            return;
        }
        let ready = self
            .inventory_auto_image
            .as_ref()
            .and_then(|s| s.wearable.as_ref())
            .is_some_and(|w| w.prepared.len() == crate::wearable_editor::affected_bakes(w.value.kind).len());
        if !ready {
            return;
        }
        let bakes = self
            .inventory_auto_image
            .as_ref()
            .and_then(|s| s.wearable.as_ref())
            .map(|w| w.prepared.iter().map(|(te, (id, _))| (*te, *id)).collect())
            .unwrap_or_default();
        self.world.preview_bakes(bakes);
        if let Some(s) = &mut self.inventory_auto_image
            && let Some(w) = &mut s.wearable
        {
            w.done = true;
        }
    }

    pub(super) fn inventory_auto_image_frame(&mut self, renderer: &mut Renderer) {
        let Some(s) = &self.inventory_auto_image else {
            return;
        };
        if s.confirming || s.saved || s.restored {
            return;
        }
        let wearable = s.wearable.is_some();
        let (yaw, zoom) = (s.yaw, s.zoom);
        if wearable {
            self.wearable_frame(renderer);
            let ready = self
                .inventory_auto_image
                .as_ref()
                .and_then(|s| s.wearable.as_ref())
                .is_some_and(|w| w.done);
            if !ready {
                return;
            }
            let iso = &mut self.world.photo_isolation;
            iso.set_picking(false);
            iso.set_mode(Mode::AvatarOnly);
            iso.set_transparent(true);
            let fov = self.camera.fov_y.max(0.1);
            if let Some(joints) = self.portrait_joints() {
                let (eye, target) = portrait::frame(&joints, portrait::Framing::Body, portrait::Angle::Front, fov);
                let base = (target - eye).normalize();
                let direction = Quat::from_rotation_z(yaw.to_radians()) * base;
                let radius = (eye - target).length() * zoom;
                let eye = target - direction * radius;
                self.camera.fixed_view = Some(FixedView {
                    eye,
                    forward: (target - eye).normalize(),
                    up: Vec3::Z,
                    fov_y: fov,
                });
            }
            if self.snapshots.drawing() {
                return;
            }
            let now = Instant::now();
            if let Some(s) = &mut self.inventory_auto_image {
                if s.shot_wanted {
                    s.shot_wanted = false;
                    s.shooting = true;
                    self.photo_job(Purpose::InventoryImage { preview: false }, renderer);
                    return;
                }
                let due = s.preview_at.is_none_or(|at| at.elapsed() >= PREVIEW_EVERY);
                if s.shot.is_none() && !s.shooting && due {
                    s.preview_at = Some(now);
                    self.photo_job(Purpose::InventoryImage { preview: true }, renderer);
                }
            }
            return;
        } else if s.attachment.is_none() {
            if let Some((index, _)) = self.world.objects.iter().find(|(_, o)| o.attachment_item_id() == Some(s.item)) {
                let root = isolate::attachment_root_of(&self.world, index);
                if let Some((_, id)) = root
                    && let Some(s) = &mut self.inventory_auto_image
                {
                    s.attachment = Some(id);
                    s.wait = None;
                }
            } else if s.wait.is_some_and(|at| at.elapsed() > DRESS_WAIT) {
                if let Some(s) = &mut self.inventory_auto_image {
                    s.error = aurora_i18n::tr!("ui-inventory-auto-image-no-object").to_owned();
                    s.wait = None;
                }
                self.end_inventory_auto_image_photo();
            }
            return;
        }
        let attachment = s.attachment.expect("attachment known");
        let Some(index) = self.world.objects.index_of_uuid(&attachment) else {
            return;
        };
        let rigged = crate::scene::Scene::skeleton_owner(&self.world, index).is_some();
        let (yaw, zoom) = (s.yaw, s.zoom);
        let now = Instant::now();
        let Some((_, object_rot, _)) = crate::scene::Scene::object_transform(&self.world, index, now, 0) else {
            return;
        };
        // `wide` is the wearer's *whole skeleton* culling sphere (root plus
        // ~2 m), so orbiting it makes the camera circle the avatar while the
        // garment hangs off to one side. Use the posed bounds of the meshes
        // in the worn linkset (not just its root prim), then fall back to
        // the prim's scale before the geometry arrives.
        let (center, radius) = self.subject_bounds(index, rigged).unwrap_or_else(|| {
            let pos = crate::scene::Scene::object_transform(&self.world, index, now, 0)
                .map(|(p, _, _)| p)
                .unwrap_or_default();
            let scale = self.world.objects.get(index).map(|o| o.scale).unwrap_or(Vec3::ONE);
            (pos, scale.abs().max_element().max(0.05) * 0.75)
        });
        let radius = radius.max(0.05);
        let fov = self.camera.fov_y.max(0.1);
        let distance = (radius / (fov * 0.5).tan() * FRAMING_MARGIN * zoom).max(0.2);
        // Look at it from the side it was authored facing. A rigged item's
        // front follows the joint it hangs from (a turned head, a pointed
        // foot), not just the avatar's root; without a pose the root's forward
        // is the fallback, and with no avatar at all the world's +Y.
        let wearer = isolate::wearer_of(&self.world, index).map(|(i, _)| i);
        let wearer_rot = wearer
            .and_then(|i| crate::scene::Scene::object_transform(&self.world, i, now, 0))
            .map(|(_, q, _)| q);
        let point_rot = if rigged {
            wearer.and_then(|i| {
                let root = self.world.objects.get(index)?;
                let wearer_obj = self.world.objects.get(i)?;
                let frame = crate::scene::anchors::attach_frame(&self.world, wearer_obj, root.attachment_point());
                (!frame.hud).then_some(frame.rot)
            })
        } else {
            None
        };
        let forward_flat = subject_front(rigged, wearer_rot, point_rot);
        let direction = if forward_flat.length_squared() > 1e-4 {
            forward_flat.normalize()
        } else {
            Vec3::Y
        };
        let direction = Quat::from_rotation_z(yaw.to_radians()) * direction;
        let eye = center + direction * distance + Vec3::Z * radius * 0.25;
        let facing = (center - eye).normalize();
        // SL prim local -Z is down. A non-rigged attachment carries the
        // attachment point's rotation; a rigged mesh is posed by the
        // skeleton and never sees that rotation.
        let up = subject_up(rigged, object_rot, wearer_rot);
        let mut up = up - facing * facing.dot(up);
        if up.length_squared() < 1e-6 {
            up = (object_rot * Vec3::X) - facing * facing.dot(object_rot * Vec3::X);
        }
        let up = up.normalize();
        self.camera.fixed_view = Some(FixedView {
            eye,
            forward: facing,
            up,
            fov_y: fov,
        });

        let iso = &mut self.world.photo_isolation;
        iso.set_picking(false);
        iso.set_mode(Mode::Attachment);
        iso.set_transparent(true);
        if !iso.is_chosen(&attachment) {
            iso.toggle(attachment);
        }

        if self.snapshots.drawing() {
            return;
        }
        if let Some(s) = &mut self.inventory_auto_image {
            if s.shot_wanted {
                s.shot_wanted = false;
                s.shooting = true;
                self.photo_job(Purpose::InventoryImage { preview: false }, renderer);
                return;
            }
            let due = s.preview_at.is_none_or(|at| at.elapsed() >= PREVIEW_EVERY);
            if s.shot.is_none() && !s.shooting && due {
                s.preview_at = Some(now);
                self.photo_job(Purpose::InventoryImage { preview: true }, renderer);
            }
        }
    }

    pub(super) fn inventory_auto_image_shoot(&mut self) {
        if let Some(s) = &mut self.inventory_auto_image
            && (s.attachment.is_some() || s.wearable.as_ref().is_some_and(|w| w.done))
        {
            s.shot_wanted = true;
        }
    }

    pub(super) fn inventory_auto_image_use(&mut self) {
        let Some(s) = self.inventory_auto_image.as_ref() else {
            return;
        };
        let Some(shot) = s.shot.as_ref() else {
            return;
        };
        // Keep the shot in the session: it is the flattened, opaque picture
        // the window keeps showing after the photo part is over.
        let (item, size, pixels) = (s.item, shot.size, shot.pixels.clone());
        let saved = self.item_photo_pixels(item, size, pixels).is_some();
        if let Some(s) = &mut self.inventory_auto_image {
            s.saved = saved;
            if !saved {
                s.error = aurora_i18n::tr!("app-inventory-thumbnail-an-edit-is-already-in-progress-or-the-item").to_owned();
            }
        }
        // The photo part is over: do not keep the scene isolated while the
        // picture is encoded and uploaded.
        self.end_inventory_auto_image_photo();
    }

    pub(super) fn inventory_auto_image_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        let Some(s) = &self.inventory_auto_image else {
            return;
        };
        let mut open = s.open;
        let item = s.item;
        let confirming = s.confirming;
        let saved = s.saved;
        let error = s.error.clone();
        let has_shot = s.shot.is_some();
        let shot = s.shot.as_ref().map(|shot| shot.texture.clone());
        let preview = s.preview.clone();
        let (mut yaw, mut zoom) = (s.yaw, s.zoom);
        let restored = s.restored;
        let name = crate::world::inventory::actions::name(&self.world.inventory, item);
        let mut start = false;
        let mut shoot = false;
        let mut use_shot = false;
        let mut retake = false;
        let mut cancel = false;
        let position = ctx.content_rect().center() - egui::vec2(170.0, 220.0);
        Floater::new(
            &format!("inventory_auto_image_{item}"),
            aurora_i18n::tr!("ui-inventory-auto-image-title"),
            position,
            Vec2::new(340.0, 440.0),
        )
        .fixed()
        .show(ctx, p, &mut open, |ui| {
            ui.label(egui::RichText::new(&name).color(p.ink));
            ui.add_space(8.0);
            let side = ui.available_width().min(288.0);
            ui.vertical_centered(|ui| {
                if let Some(texture) = shot.as_ref().or(preview.as_ref()) {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(side), egui::Sense::hover());
                    ui.painter().image(
                        texture.id(),
                        rect,
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                        crate::ui::widgets::PICTURE_TINT,
                    );
                } else {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(side), egui::Sense::hover());
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        if confirming {
                            aurora_i18n::tr!("ui-inventory-auto-image-local")
                        } else {
                            aurora_i18n::tr!("ui-inventory-auto-image-preparing")
                        },
                        egui::FontId::proportional(13.0),
                        p.muted,
                    );
                }
            });
            ui.add_space(8.0);
            let adjustable = !confirming && !saved && !restored;
            ui.horizontal(|ui| {
                let half = ui.available_width() / 2.0;
                ui.allocate_ui(egui::vec2(half, 44.0), |ui| {
                    ui.vertical(|ui| {
                        ui.label(aurora_i18n::tr!("ui-inventory-auto-image-rotate"));
                        ui.add_enabled(adjustable, egui::Slider::new(&mut yaw, -180.0..=180.0).suffix("°"));
                    });
                });
                ui.allocate_ui(egui::vec2(half, 44.0), |ui| {
                    ui.vertical(|ui| {
                        ui.label(aurora_i18n::tr!("ui-inventory-auto-image-zoom"));
                        ui.add_enabled(adjustable, egui::Slider::new(&mut zoom, 0.5..=3.0).suffix("×").step_by(0.05));
                    });
                });
            });
            ui.add_space(8.0);
            if !error.is_empty() {
                ui.label(egui::RichText::new(&error).color(p.warn));
            }
            if saved {
                ui.label(egui::RichText::new(aurora_i18n::tr!("ui-inventory-auto-image-saved")).color(p.success));
            }
            ui.horizontal(|ui| {
                if confirming {
                    start = primary_button_sized(ui, p, aurora_i18n::tr!("ui-inventory-auto-image-wear"), Vec2::new(150.0, 30.0)).clicked();
                } else if !saved {
                    shoot = flat_button(ui, p, aurora_i18n::tr!("ui-appearance-portrait-take")).clicked();
                    if has_shot {
                        use_shot =
                            primary_button_sized(ui, p, aurora_i18n::tr!("ui-appearance-portrait-use"), Vec2::new(120.0, 30.0)).clicked();
                        retake = flat_button(ui, p, aurora_i18n::tr!("ui-appearance-portrait-retake")).clicked();
                    }
                }
                let close_label = if saved {
                    aurora_i18n::tr!("common-close")
                } else {
                    aurora_i18n::tr!("common-cancel")
                };
                if flat_button(ui, p, close_label).clicked() {
                    cancel = true;
                }
            });
        });
        if cancel {
            open = false;
        }
        if let Some(s) = &mut self.inventory_auto_image {
            s.open = open;
            if s.yaw != yaw || s.zoom != zoom {
                s.yaw = yaw;
                s.zoom = zoom;
                // The next preview follows the new view right away.
                s.preview_at = None;
            }
        }
        if !open {
            self.close_inventory_auto_image();
            return;
        }
        if start {
            self.begin_inventory_auto_image();
        }
        if shoot {
            self.inventory_auto_image_shoot();
        }
        if use_shot {
            self.inventory_auto_image_use();
        }
        if retake && let Some(s) = &mut self.inventory_auto_image {
            s.shot = None;
        }
    }
}

const WINDOW: &str = "inventory_auto_image";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachable_objects_system_wearables_and_nothing_else_are_photographed() {
        let mut item = aurora_net::inventory::InvItem {
            asset_type: 6,
            ..Default::default()
        };
        assert!(eligible(&item));
        for kind in [5, 13] {
            item.asset_type = kind;
            assert!(eligible(&item), "asset type {kind}");
        }
        for other in [0, 7, 15, 20, 24, 49, 57] {
            item.asset_type = other;
            assert!(!eligible(&item), "asset type {other}");
        }
    }

    #[test]
    fn a_non_rigged_photo_uses_the_attachment_points_down_axis() {
        // A point tilted 90 degrees about X turns the object's local +Z
        // towards -Y: the photo's up must follow it so its local -Z is down.
        let point = Quat::from_euler(glam::EulerRot::XYZ, std::f32::consts::FRAC_PI_2, 0.0, 0.0);
        let up = subject_up(false, point, Some(Quat::IDENTITY));
        assert!((up - Vec3::new(0.0, -1.0, 0.0)).length() < 1e-5, "{up:?}");
    }

    #[test]
    fn a_rigged_photo_levels_from_the_skeleton_and_ignores_the_object() {
        let wearer = Quat::from_euler(glam::EulerRot::XYZ, std::f32::consts::FRAC_PI_2, 0.0, 0.0);
        let object = Quat::from_euler(glam::EulerRot::XYZ, 0.0, 0.0, std::f32::consts::FRAC_PI_2);
        let up = subject_up(true, object, Some(wearer));
        assert!((up - Vec3::new(0.0, -1.0, 0.0)).length() < 1e-5, "{up:?}");
        assert!(subject_up(true, object, None) == Vec3::Z);
    }

    #[test]
    fn a_rigged_photo_faces_the_attachment_points_joint() {
        // The head turned a quarter turn right takes the garment's front with
        // it, even though the avatar's root still faces +Y.
        let point = Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2);
        let front = subject_front(true, Some(Quat::IDENTITY), Some(point));
        assert!((front - Vec3::X).length() < 1e-5, "{front:?}");
    }

    #[test]
    fn the_joint_front_composes_with_the_avatars_facing() {
        let quarter = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let front = subject_front(true, Some(quarter), Some(quarter));
        assert!((front - Vec3::NEG_Y).length() < 1e-5, "{front:?}");
    }

    #[test]
    fn an_unknown_joint_or_a_non_rigged_prim_keeps_the_avatar_forward() {
        let avatar = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let point = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let expected = Vec3::NEG_X;
        let no_joint = subject_front(true, Some(avatar), None);
        assert!((no_joint - expected).length() < 1e-5, "{no_joint:?}");
        let non_rigged = subject_front(false, Some(avatar), Some(point));
        assert!((non_rigged - expected).length() < 1e-5, "{non_rigged:?}");
    }

    #[test]
    fn a_joint_pointing_straight_up_leaves_no_forward_to_follow() {
        // The preview then falls back to the world's +Y, as before.
        let point = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
        let front = subject_front(true, Some(Quat::IDENTITY), Some(point));
        assert!(front.length() < 1e-5, "{front:?}");
    }
}
