//! Automatic picture of an attachable inventory item (Aurora addition).
//!
//! The Legacy Viewer's Change item thumbnail takes a snapshot of whatever
//! is on screen. Here the item is worn for the picture, the isolation shows
//! that attachment alone (its wearer is only unseen, so a rigged garment
//! still has a posed skeleton and a FULL cull state), and the square photo
//! becomes the item's picture through the ordinary thumbnail upload. The
//! camera is levelled from the attachment's own down axis. A rigged mesh is
//! photographed from its bind frame's front, turned by the joint or point
//! that carries it; a non-rigged prim keeps the attachment point's rotation.
//!
//! Nothing here changes the grid: the wear is the same as double-clicking
//! the item, and the previous outfit is put back when it was not already
//! worn (the window asks for consent first).

use super::App;
use super::snapshot::Purpose;
use crate::camera::FixedView;
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

const WINDOW: &str = "inventory_auto_image";

/// How long to wait for the simulator to build the worn object.
const DRESS_WAIT: Duration = Duration::from_secs(15);
/// The camera's margin around the attachment's bounds.
const FRAMING_MARGIN: f32 = 1.5;
/// The preview is drawn again this often while it is open.
const PREVIEW_EVERY: Duration = Duration::from_millis(500);
/// Give up on a wearable bake after this long instead of sitting on
/// "preparing" forever when a texture never arrives.
const WEARABLE_WAIT: Duration = Duration::from_secs(30);
/// Fraction of the measured centring error applied to the pan each preview.
/// The error is measured from an image drawn with the current pan, so it is
/// the *remaining* offset; adding it converges smoothly to the real centre.
const PAN_GAIN: f32 = 0.7;
/// Ignore a residual smaller than this fraction of a frame: once the mesh and
/// its textures have loaded the alpha edge keeps shifting by a pixel or two,
/// and chasing that makes a centred subject jiggle forever.
const PAN_DEAD_ZONE: f32 = 0.015;
/// Stop panning after this many previews. A subject too large for the frame
/// (a whole body) has no stable visible centre, so an endless search for it
/// would drift; by now the ordinary assets have loaded and the pan has settled.
const MAX_PAN_STEPS: u8 = 12;

/// Inventory asset types the automatic picture handles (LLAssetType, as in
/// `build::contents_drop`): an attachable object, a clothing layer or a body
/// part. Anything else (textures, sounds, scripts…) is not worn for a photo.
pub(crate) const AT_OBJECT: i32 = 6;
pub(crate) const AT_CLOTHING: i32 = 5;
pub(crate) const AT_BODYPART: i32 = 13;

/// Whether the item can be photographed: an attachable object, which is worn
/// for the picture, or a system wearable, whose bakes are composed locally.
pub(crate) fn auto_image_eligible(asset_type: i32) -> bool {
    matches!(asset_type, AT_OBJECT | AT_CLOTHING | AT_BODYPART)
}

fn eligible(item: &aurora_net::inventory::InvItem) -> bool {
    auto_image_eligible(item.asset_type)
}

/// A system wearable photographed from locally composed bakes on a white
/// base: the avatar is not undressed and nothing is sent to the grid.
struct WearablePhoto {
    value: Wearable,
    /// Texture ids the bake needs. The pixel sources are refreshed from the
    /// texture system every frame: the asset arrives before its textures have
    /// decoded, and a source captured then never becomes ready.
    sources: Vec<Uuid>,
    /// The ids have been acquired from the texture streamer, so they are
    /// downloading and must be released when the picture is over.
    acquired: bool,
    rx: Option<crossbeam_channel::Receiver<BakeReply>>,
    prepared: HashMap<usize, (Uuid, u32)>,
    started: Instant,
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

/// The front of a rigged mesh in its bind frame. Second Life's skeleton binds
/// garments facing +X, a quarter turn from the avatar's +Y; photographing from
/// +Y shows them side-on (the old default that needed Rotate -90°).
const RIGGED_BIND_FRONT: Vec3 = Vec3::X;

/// World direction the worn item was authored facing, flattened so the camera
/// stays level. A rigged mesh is posed in the avatar's bind frame, so the
/// chain turns whichever bone or point carries it to that frame's +X front:
/// a body-facing skinned joint first (a limb bone's +Y runs along the limb,
/// so it is ignored), then the attachment point's posed joint
/// (`anchors::attach_frame`), then the avatar's root — a garment weighted to
/// the legs still faces the way its wearer does. The caller falls back to the
/// bind front if even that points straight up. A non-rigged prim keeps the
/// root's forward: it already carries the point's own rotation.
fn subject_front(rigged: bool, wearer_rot: Option<Quat>, point_rot: Option<Quat>, dominant_front: Option<Vec3>) -> Vec3 {
    let wearer_rot = wearer_rot.unwrap_or(Quat::IDENTITY);
    let flat = |v: Vec3| Vec3::new(v.x, v.y, 0.0);
    if !rigged {
        return flat(wearer_rot * Vec3::Y);
    }
    // A garment carried by a pelvis/torso/chest bone faces the way its wearer
    // faces: the bone's local axis is a bind convention, not a front. Use the
    // bind front directly; a steep axis discards that hint.
    if let Some(front) = dominant_front
        && front.is_finite()
        && flat(front).length() > 0.25
    {
        return flat(wearer_rot * RIGGED_BIND_FRONT);
    }
    if let Some(point) = point_rot {
        let front = flat(wearer_rot * point * RIGGED_BIND_FRONT);
        if front.length_squared() > 1e-4 {
            return front;
        }
    }
    flat(wearer_rot * RIGGED_BIND_FRONT)
}

/// Alpha-weighted centre of a preview's visible pixels, in image coordinates
/// (0..1, y down). The transparent backdrop tells us what the camera actually
/// drew, which geometry bounds cannot when skinning skews them.
fn alpha_centroid(pixels: &[u8], width: u32, height: u32) -> Option<(f32, f32)> {
    if width == 0 || height == 0 || pixels.len() < width as usize * height as usize * 4 {
        return None;
    }
    let (mut sum, mut sx, mut sy) = (0.0f64, 0.0f64, 0.0f64);
    for y in 0..height as usize {
        for x in 0..width as usize {
            let alpha = pixels[(y * width as usize + x) * 4 + 3] as f64;
            if alpha > 8.0 {
                sum += alpha;
                sx += x as f64 * alpha;
                sy += y as f64 * alpha;
            }
        }
    }
    (sum > 0.0).then(|| {
        (
            (((sx / sum) + 0.5) / width as f64) as f32,
            (((sy / sum) + 0.5) / height as f64) as f32,
        )
    })
}

/// Advance the camera-space pan (right, up) from a preview's measured centre.
/// The centroid comes from an image drawn with the current pan, so it is the
/// remaining error, not an absolute position: using it as the latter makes
/// the camera alternate around a point only part-way to the centre. Adding
/// the residual converges smoothly and reaches it. The dead zone stops a
/// centred subject jiggling as textures load, and the cap bounds a stray
/// opaque pixel or a subject too large for the frame.
fn advance_pan(pan: (f32, f32), centroid: Option<(f32, f32)>, span: f32) -> (f32, f32) {
    let Some((cx, cy)) = centroid else {
        return pan;
    };
    let (dx, dy) = (cx - 0.5, cy - 0.5);
    if dx.hypot(dy) < PAN_DEAD_ZONE {
        return pan;
    }
    let (mut x, mut y) = pan;
    x += dx * span * PAN_GAIN;
    y += -dy * span * PAN_GAIN;
    let len = x.hypot(y);
    if span > 0.0 && len > span {
        let k = span / len;
        x *= k;
        y *= k;
    }
    (x, y)
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
    /// Geometry of the whole worn item, not just its root prim: a garment is
    /// often a linkset whose visible panels are children of a scripted or
    /// invisible root. Centre-cropping the root alone leaves the picture
    /// badly off centre and makes rotation swing around the wrong point. One
    /// pass returns both the bounds and the dominant skinned joint's front,
    /// since both walk the same linkset.
    fn subject_geometry(&self, root: usize, rigged: bool) -> (Option<(Vec3, f32)>, Option<Vec3>) {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        let mut best: Option<(f32, Vec3)> = None;
        for (idx, _) in self.world.objects.iter() {
            if !under_root(&self.world, idx, root) {
                continue;
            }
            let g = self.scene.gpu.get(idx).filter(|g| !g.faces.is_empty());
            let bounds = if rigged {
                self.scene.posed_object_bounds(&self.world, idx).map(|(_, radius)| {
                    // A garment weighted to many joints has large, heavily
                    // overlapping joint boxes: their union makes the subject
                    // several times bigger than it is and skews its centre.
                    // Its own bind-space box bounds the size, and its centre
                    // posed by the largest joint follows the visible mesh.
                    let tight = self.scene.bind_radius(idx).map(|r| r * 1.25).unwrap_or(f32::INFINITY);
                    let center = self
                        .scene
                        .posed_subject_center(&self.world, idx)
                        .unwrap_or_else(|| self.scene.gpu.get(idx).map_or(Vec3::ZERO, |g| g.center));
                    (center, radius.min(tight))
                })
            } else {
                None
            }
            .or_else(|| g.map(|g| (g.center, g.radius)));
            if let Some((center, radius)) = bounds {
                let span = Vec3::splat(radius);
                min = min.min(center - span);
                max = max.max(center + span);
            }
            // The visible panels of a garment are often children of an
            // invisible root: compare the meshes' extents on their bones.
            if rigged
                && let Some((extent, front)) = self.scene.posed_joint_front(&self.world, idx)
                && best.as_ref().is_none_or(|(b, _)| extent > *b)
            {
                best = Some((extent, front));
            }
        }
        let bounds = min.x.is_finite().then(|| ((min + max) * 0.5, (max - min).length() * 0.5));
        (bounds, best.map(|(_, front)| front))
    }
}

/// The photo awaiting "Use".
struct Shot {
    size: (u32, u32),
    pixels: Vec<u8>,
    texture: egui::TextureHandle,
}

/// What the window is waiting for, shown under the picture until it is ready.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    /// The consent step before anything is worn.
    Local,
    /// Waiting for the system wearable's asset from the grid.
    Asset,
    /// Waiting for its textures: (ready, total).
    Textures(usize, usize),
    /// Composing the local bake.
    Bake,
    /// Waiting for the simulator to build the attachment.
    Object,
    /// A preview is on screen; take the picture when ready.
    Ready,
}

/// Whether a flattened picture is essentially one flat colour: a failed bake
/// can produce a clean but empty image that would otherwise be uploaded.
fn looks_blank(pixels: &[u8]) -> bool {
    let mut lo = [255u8; 3];
    let mut hi = [0u8; 3];
    for px in pixels.as_chunks::<4>().0 {
        for c in 0..3 {
            lo[c] = lo[c].min(px[c]);
            hi[c] = hi[c].max(px[c]);
        }
    }
    (0..3).all(|c| hi[c].saturating_sub(lo[c]) < 12)
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
    /// The automatic fit has not been adjusted by the user yet.
    zoom_default: bool,
    preview_at: Option<Instant>,
    preview: Option<egui::TextureHandle>,
    /// The last preview's pixels, to pan the camera onto what is actually
    /// visible (the geometric bounds can be skewed by heavy skinning).
    preview_pixels: Option<(Vec<u8>, u32, u32)>,
    /// Accumulated camera-space pan (right, up) in world units.
    pan_x: f32,
    pan_y: f32,
    /// Set when a new preview's pixels arrive; the pan advances only once
    /// per preview, not once per rendered frame.
    pan_dirty: bool,
    /// Pan steps taken so far; after the cap the camera stops re-centring.
    pan_steps: u8,
    /// What the window is waiting for, shown until a preview is drawn.
    stage: Stage,
    /// The last photo came out nearly uniform and is probably empty.
    blank: bool,
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
        let wearable = matches!(it.asset_type, AT_CLOTHING | AT_BODYPART);
        let confirming = !was_worn && !wearable;
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
            zoom_default: true,
            preview_at: None,
            preview: None,
            preview_pixels: None,
            pan_x: 0.0,
            pan_y: 0.0,
            pan_dirty: false,
            pan_steps: 0,
            stage: if confirming {
                Stage::Local
            } else if wearable {
                Stage::Asset
            } else {
                Stage::Object
            },
            blank: false,
            shot: None,
            shot_wanted: false,
            shooting: false,
            error: String::new(),
            saved: false,
            restored: false,
            confirming,
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
        let wearable_ids = s.wearable.as_ref().map(|w| w.sources.clone()).unwrap_or_default();
        if let Some(pose) = pose {
            let id = self.world.agent_id;
            if self.scene.poser.get(&id).is_some_and(|held| Arc::ptr_eq(held, &pose)) {
                self.scene.poser.remove(&id);
            }
        }
        if had_wearable {
            for id in &wearable_ids {
                self.scene.textures.release(id);
            }
            self.world.preview_bakes(HashMap::new());
        }
        self.camera = camera;
        self.camera.fixed_view = fixed;
        // Bumped past the current version: the scene re-flags every object.
        let version = self.world.photo_isolation.version;
        self.world.photo_isolation = isolation;
        self.world.photo_isolation.version = version.wrapping_add(1);
        // A system wearable is never worn for the picture (its bakes are
        // composed locally), so there is nothing to take off even when the
        // resident already had that wearable on. `worn_attachment_items`
        // only sees attachment objects, not system layers.
        if !was_worn && !confirming && !had_wearable && !self.demo {
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
        let background = self.settings.auto_image_background;
        let Some(s) = &mut self.inventory_auto_image else {
            return;
        };
        if preview {
            s.preview_pixels = Some((pixels.clone(), size.0, size.1));
            s.pan_dirty = true;
            let image = egui::ColorImage::from_rgba_unmultiplied([size.0 as usize, size.1 as usize], &pixels);
            s.preview = Some(
                self.egui_ctx
                    .load_texture("inventory_auto_image_preview", image, egui::TextureOptions::LINEAR),
            );
            return;
        }
        if cutout && background != crate::settings::AutoImageBackground::Transparent {
            let rgb = match background {
                crate::settings::AutoImageBackground::White => [255, 255, 255],
                crate::settings::AutoImageBackground::Grey | crate::settings::AutoImageBackground::Transparent => {
                    portrait::backdrop_rgb(portrait::Backdrop::Grey).unwrap_or([64, 64, 64])
                }
            };
            portrait::flatten(&mut pixels, rgb);
        }
        s.blank = looks_blank(&pixels);
        s.stage = Stage::Ready;
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
        log::info!(
            "inventory auto image: wearable {} kind {}, {} texture source(s)",
            item,
            value.kind,
            ids.len()
        );
        if let Some(s) = &mut self.inventory_auto_image {
            s.wearable = Some(WearablePhoto {
                value,
                sources: ids,
                acquired: false,
                rx: None,
                prepared: HashMap::new(),
                started: Instant::now(),
                done: false,
            });
        }
    }

    /// Drive the bake job and apply its textures to our own avatar.
    fn wearable_frame(&mut self, renderer: &mut Renderer) {
        let (rx, ids, kind, value, acquired) = {
            let Some(s) = self.inventory_auto_image.as_ref() else {
                return;
            };
            let Some(w) = s.wearable.as_ref() else {
                return;
            };
            (w.rx.clone(), w.sources.clone(), w.value.kind, w.value.clone(), w.acquired)
        };
        // The wearable's textures may never have been requested when it was
        // not worn: note_usage alone cannot create an entry, so acquire them
        // to start the download. This is what the wearable editor's preview
        // does for the same sources.
        if !acquired {
            for id in &ids {
                self.scene.textures.acquire(renderer, *id, crate::scene::textures::TexSource::Asset);
            }
            if let Some(s) = &mut self.inventory_auto_image
                && let Some(w) = &mut s.wearable
            {
                w.acquired = true;
            }
        }
        // Rebuild the sources from the texture system every frame: the asset
        // arrives before its textures are decoded, and a snapshot taken then
        // would never report itself ready, leaving the picture "preparing".
        let mut sources = HashMap::new();
        for id in &ids {
            self.scene.textures.note_usage_fixed(id, f32::MAX);
            sources.insert(*id, self.scene.textures.preview_source(id));
        }
        let ready_count = ids.iter().filter(|id| sources.get(*id).is_some_and(|s| s.bake_ready(**id))).count();
        let ready = ready_count == ids.len();
        if rx.is_none() {
            if !ready {
                if let Some(s) = &mut self.inventory_auto_image {
                    s.stage = Stage::Textures(ready_count, ids.len());
                }
                return;
            }
            if let Some(s) = &mut self.inventory_auto_image {
                s.stage = Stage::Bake;
            }
            log::info!("inventory auto image: starting wearable bake with {} source(s)", ids.len());
            let Some(pool) = self.scene.textures.staging.clone() else {
                log::warn!("inventory auto image: no staging pool for the wearable bake");
                return;
            };
            let lib = self.world.avatar_lib.clone();
            let base = self.world.server_visual_params().to_vec();
            let (tx, rx) = crossbeam_channel::unbounded();
            let token = Arc::new(AtomicU64::new(1));
            self.scene.jobs.execute(move || {
                match bake_preview::prepare(
                    1,
                    &token,
                    &pool,
                    &lib,
                    &base,
                    crate::wearable_editor::affected_bakes(kind),
                    vec![value],
                    sources,
                    &tx,
                    None,
                    true,
                ) {
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
        let Some(rx) = rx.as_ref() else {
            return;
        };
        let mut done = false;
        let mut failed = None;
        while let Ok(reply) = rx.try_recv() {
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
        let expected = self
            .inventory_auto_image
            .as_ref()
            .and_then(|s| s.wearable.as_ref())
            .map(|w| crate::wearable_editor::affected_bakes(w.value.kind).len())
            .unwrap_or(0);
        let prepared = self
            .inventory_auto_image
            .as_ref()
            .and_then(|s| s.wearable.as_ref())
            .map(|w| w.prepared.len())
            .unwrap_or(0);
        log::info!("inventory auto image: wearable bake done, {prepared} of {expected} bake(s) ready");
        if prepared != expected {
            return;
        }
        let mut bakes: HashMap<usize, Uuid> = self
            .inventory_auto_image
            .as_ref()
            .and_then(|s| s.wearable.as_ref())
            .map(|w| w.prepared.iter().map(|(te, (id, _))| (*te, *id)).collect())
            .unwrap_or_default();
        // The BOM meshes stay in the picture as the white mannequin, but only
        // this item's bake may show. Slots displayed by a BOM mesh and the AUX
        // slots (which have no system body part) go white so the mesh remains.
        // The other system bakes must hide their part: a skirt is drawn as
        // soon as it has any bake that is not the invisible one.
        let bom = self
            .world
            .objects
            .index_of_uuid(&self.world.agent_id)
            .map(|avatar| self.scene.bom_mask(&self.world, avatar))
            .unwrap_or(0);
        for (slot, _, _) in &crate::scene::avatar::BAKES {
            if bakes.contains_key(slot) {
                continue;
            }
            let keep_mesh = *slot >= 40 || bom & (1u64 << (*slot).min(63)) != 0;
            bakes.insert(
                *slot,
                if keep_mesh {
                    aurora_prim::te::BLANK_TEXTURE
                } else {
                    crate::scene::avatar::IMG_INVISIBLE
                },
            );
        }
        self.world.preview_bakes(bakes);
        if let Some(s) = &mut self.inventory_auto_image
            && let Some(w) = &mut s.wearable
        {
            w.done = true;
            s.stage = Stage::Ready;
        }
    }

    /// Submit the chosen shot, or the next timed preview, at most one a frame.
    fn inventory_auto_image_photo_job(&mut self, renderer: &mut Renderer, now: Instant) {
        let Some(s) = &mut self.inventory_auto_image else {
            return;
        };
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
            let timed_out = self
                .inventory_auto_image
                .as_ref()
                .and_then(|s| s.wearable.as_ref())
                .is_some_and(|w| !w.done && w.started.elapsed() > WEARABLE_WAIT);
            if timed_out {
                log::warn!("inventory auto image: wearable picture timed out after {WEARABLE_WAIT:?}");
                if let Some(s) = &mut self.inventory_auto_image {
                    s.error = aurora_i18n::tr!("ui-inventory-auto-image-timeout").to_owned();
                }
                self.end_inventory_auto_image_photo();
                return;
            }
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
            self.inventory_auto_image_photo_job(renderer, Instant::now());
            return;
        } else if s.attachment.is_none() {
            let found = self
                .world
                .objects
                .iter()
                .find(|(_, o)| o.attachment_item_id() == Some(s.item))
                .map(|(index, _)| index);
            if let Some(index) = found {
                // HUD attachments live in screen space and are not part of
                // the world picture: refuse before a shot can be taken.
                if self.world.objects.worn_on_hud(index) {
                    if let Some(s) = &mut self.inventory_auto_image {
                        s.error = aurora_i18n::tr!("ui-inventory-auto-image-hud").to_owned();
                        s.wait = None;
                    }
                    self.end_inventory_auto_image_photo();
                    return;
                }
                let root = isolate::attachment_root_of(&self.world, index);
                // A rigged garment is much smaller than the skeleton that
                // poses it: start the camera closer on its first framing.
                let rigged = crate::scene::Scene::skeleton_owner(&self.world, index).is_some();
                if let Some((_, id)) = root
                    && let Some(s) = &mut self.inventory_auto_image
                {
                    s.attachment = Some(id);
                    s.wait = None;
                    s.stage = Stage::Ready;
                    if rigged && s.zoom_default {
                        s.zoom = 0.6;
                        s.zoom_default = false;
                    }
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
        let Some(attachment) = s.attachment else {
            return;
        };
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
        let (geometry, dominant_front) = self.subject_geometry(index, rigged);
        let dominant_front = if rigged { dominant_front } else { None };
        let (center, radius) = geometry.unwrap_or_else(|| {
            let pos = crate::scene::Scene::object_transform(&self.world, index, now, 0)
                .map(|(p, _, _)| p)
                .unwrap_or_default();
            let scale = self.world.objects.get(index).map(|o| o.scale).unwrap_or(Vec3::ONE);
            (pos, scale.abs().max_element().max(0.05) * 0.75)
        });
        let radius = radius.max(0.05);
        let fov = self.camera.fov_y.max(0.1);
        // The posed bounds are generous for skinned meshes; a rigid attachment
        // keeps the roomier margin so it cannot touch the picture's edges.
        let margin = if rigged { 1.0 } else { FRAMING_MARGIN };
        let distance = (radius / (fov * 0.5).tan() * margin * zoom).max(0.2);
        // Look at it from the side it was authored facing. `subject_front`
        // turns the bone or attachment point that carries a rigged mesh to
        // the bind front; a non-rigged prim keeps the point's own forward.
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
        let forward_flat = subject_front(rigged, wearer_rot, point_rot, dominant_front);
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
        // Pan onto what the last preview actually drew. Geometric bounds can
        // be skewed by overlapping joint boxes; the alpha mask cannot. The
        // mask is measured from the already-panned camera, so its offset is
        // the remaining error and is accumulated, not used as a new position.
        // Once the assets have loaded and the pan has settled, stop: a whole
        // body that cannot fit the frame has no stable centre to chase.
        let span = 2.0 * distance * (fov * 0.5).tan();
        let auto_centre = self.settings.auto_image_centre;
        let pan = if auto_centre && s.pan_dirty && s.pan_steps < MAX_PAN_STEPS {
            let centroid = s
                .preview_pixels
                .as_ref()
                .and_then(|(pixels, width, height)| alpha_centroid(pixels, *width, *height));
            let next = advance_pan((s.pan_x, s.pan_y), centroid, span);
            if let Some(s) = &mut self.inventory_auto_image {
                if next != (s.pan_x, s.pan_y) {
                    s.pan_steps += 1;
                }
                s.pan_x = next.0;
                s.pan_y = next.1;
                s.pan_dirty = false;
            }
            next
        } else {
            (s.pan_x, s.pan_y)
        };
        let right = facing.cross(up).normalize();
        let eye = eye + right * pan.0 + up * pan.1;
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
        self.inventory_auto_image_photo_job(renderer, now);
    }

    /// Reopen the same item so a failed asset or bake can be tried again
    /// without leaving the picture editor.
    fn retry_inventory_auto_image(&mut self) {
        let Some(item) = self.inventory_auto_image.as_ref().map(|s| s.item) else {
            return;
        };
        self.open_inventory_auto_image(item);
    }

    /// Put Rotate, Zoom and the automatic pan back to their starting values.
    fn reset_inventory_auto_image_view(&mut self) {
        let rigged = self
            .inventory_auto_image
            .as_ref()
            .and_then(|s| s.attachment)
            .and_then(|id| self.world.objects.index_of_uuid(&id))
            .is_some_and(|i| crate::scene::Scene::skeleton_owner(&self.world, i).is_some());
        let Some(s) = &mut self.inventory_auto_image else {
            return;
        };
        s.yaw = 0.0;
        s.zoom = if rigged { 0.6 } else { 1.0 };
        s.zoom_default = true;
        s.pan_x = 0.0;
        s.pan_y = 0.0;
        s.pan_dirty = false;
        s.pan_steps = 0;
        s.preview_at = None;
    }

    /// Turn the item to one of the four cardinal views.
    fn inventory_auto_image_view(&mut self, yaw: f32) {
        if let Some(s) = &mut self.inventory_auto_image {
            s.yaw = yaw;
            s.preview_at = None;
        }
    }

    pub(super) fn inventory_auto_image_shoot(&mut self) {
        // Final guard: the frame already refuses HUD attachments with an
        // error as soon as the worn object is identified, but the Take button
        // stays reachable until the session is restored — never photograph
        // through that path either.
        if let Some(s) = &mut self.inventory_auto_image
            && !s.restored
            && s.attachment
                .and_then(|id| self.world.objects.index_of_uuid(&id))
                .is_none_or(|index| !self.world.objects.worn_on_hud(index))
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
        let blank = s.blank;
        let shot = s.shot.as_ref().map(|shot| shot.texture.clone());
        let preview = s.preview.clone();
        let (mut yaw, mut zoom) = (s.yaw, s.zoom);
        let restored = s.restored;
        let stage = s.stage;
        let name = crate::world::inventory::actions::name(&self.world.inventory, item);
        let current_asset = self.world.inventory.items.get(&item).map(|it| it.thumbnail).unwrap_or_default();
        let current_texture = if current_asset.is_nil() {
            None
        } else {
            self.ui_images.get(&current_asset).cloned()
        };
        let status = match stage {
            Stage::Local => aurora_i18n::tr!("ui-inventory-auto-image-local").to_owned(),
            Stage::Asset => aurora_i18n::tr!("ui-inventory-auto-image-stage-asset").to_owned(),
            Stage::Textures(n, m) => aurora_i18n::tr!("ui-inventory-auto-image-stage-textures", n = n, m = m),
            Stage::Bake => aurora_i18n::tr!("ui-inventory-auto-image-stage-bake").to_owned(),
            Stage::Object => aurora_i18n::tr!("ui-inventory-auto-image-stage-object").to_owned(),
            Stage::Ready => aurora_i18n::tr!("ui-inventory-auto-image-stage-ready").to_owned(),
        };
        let mut centre = self.settings.auto_image_centre;
        let mut background = self.settings.auto_image_background;
        let mut start = false;
        let mut shoot = false;
        let mut use_shot = false;
        let mut retake = false;
        let mut retry = false;
        let mut reset = false;
        let mut view: Option<f32> = None;
        let mut cancel = false;
        let position = ctx.content_rect().center() - egui::vec2(180.0, 280.0);
        Floater::new(
            &format!("inventory_auto_image_{item}"),
            aurora_i18n::tr!("ui-inventory-auto-image-title"),
            position,
            Vec2::new(360.0, 560.0),
        )
        .fixed()
        .show(ctx, p, &mut open, |ui| {
            ui.label(egui::RichText::new(&name).color(p.ink));
            ui.add_space(6.0);
            let side = ui.available_width().min(240.0);
            ui.vertical_centered(|ui| {
                if let Some(texture) = shot.as_ref().or(preview.as_ref()) {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(side), egui::Sense::hover());
                    ui.painter().image(
                        texture.id(),
                        rect,
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                        crate::ui::widgets::PICTURE_TINT,
                    );
                    ui.label(egui::RichText::new(&status).color(p.muted));
                } else {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(side), egui::Sense::hover());
                    // The consent message is a paragraph: wrap it to the
                    // picture's width rather than drawing one clipped line.
                    let galley =
                        egui::WidgetText::from(&status).into_galley(ui, Some(egui::TextWrapMode::Wrap), side - 8.0, egui::TextStyle::Body);
                    ui.painter().galley(rect.center() - galley.size() * 0.5, galley, p.muted);
                }
            });
            if !current_asset.is_nil() {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(aurora_i18n::tr!("ui-inventory-auto-image-current")).color(p.muted));
                    if let Some(texture) = &current_texture {
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(36.0), egui::Sense::hover());
                        ui.painter().image(
                            texture.id(),
                            rect,
                            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                            crate::ui::widgets::PICTURE_TINT,
                        );
                    }
                });
            }
            ui.add_space(4.0);
            let adjustable = !confirming && !saved && !restored;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let button = Vec2::new(50.0, 22.0);
                use crate::ui::widgets::flat_button_sized;
                if flat_button_sized(ui, p, aurora_i18n::tr!("ui-inventory-auto-image-front"), button).clicked() {
                    view = Some(0.0);
                }
                if flat_button_sized(ui, p, aurora_i18n::tr!("ui-inventory-auto-image-back"), button).clicked() {
                    view = Some(180.0);
                }
                if flat_button_sized(ui, p, aurora_i18n::tr!("ui-inventory-auto-image-left"), button).clicked() {
                    view = Some(-90.0);
                }
                if flat_button_sized(ui, p, aurora_i18n::tr!("ui-inventory-auto-image-right"), button).clicked() {
                    view = Some(90.0);
                }
                if flat_button_sized(ui, p, aurora_i18n::tr!("ui-inventory-auto-image-reset"), Vec2::new(96.0, 22.0)).clicked() {
                    reset = true;
                }
            });
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
            if !confirming {
                ui.add_space(2.0);
                crate::ui::widgets::switch_labeled(ui, p, &mut centre, aurora_i18n::tr!("ui-inventory-auto-image-centre"));
            }
            ui.horizontal(|ui| {
                use crate::ui::widgets::radio_value;
                ui.label(egui::RichText::new(aurora_i18n::tr!("ui-inventory-auto-image-background")).color(p.muted));
                radio_value(
                    ui,
                    p,
                    &mut background,
                    crate::settings::AutoImageBackground::Grey,
                    aurora_i18n::tr!("ui-inventory-auto-image-background-grey"),
                );
                radio_value(
                    ui,
                    p,
                    &mut background,
                    crate::settings::AutoImageBackground::White,
                    aurora_i18n::tr!("ui-inventory-auto-image-background-white"),
                );
                radio_value(
                    ui,
                    p,
                    &mut background,
                    crate::settings::AutoImageBackground::Transparent,
                    aurora_i18n::tr!("ui-inventory-auto-image-background-transparent"),
                );
            });
            ui.add_space(4.0);
            if has_shot && blank {
                ui.label(egui::RichText::new(aurora_i18n::tr!("ui-inventory-auto-image-blank")).color(p.warn));
            }
            if !error.is_empty() {
                ui.label(egui::RichText::new(&error).color(p.warn));
            }
            if saved {
                ui.label(egui::RichText::new(aurora_i18n::tr!("ui-inventory-auto-image-saved")).color(p.success));
            }
            ui.horizontal(|ui| {
                if confirming {
                    start = primary_button_sized(ui, p, aurora_i18n::tr!("ui-inventory-auto-image-wear"), Vec2::new(150.0, 30.0)).clicked();
                } else if !saved && !restored {
                    shoot = flat_button(ui, p, aurora_i18n::tr!("ui-appearance-portrait-take")).clicked();
                    if has_shot {
                        use_shot =
                            primary_button_sized(ui, p, aurora_i18n::tr!("ui-appearance-portrait-use"), Vec2::new(120.0, 30.0)).clicked();
                        retake = flat_button(ui, p, aurora_i18n::tr!("ui-appearance-portrait-retake")).clicked();
                    }
                    if !error.is_empty() {
                        retry = flat_button(ui, p, aurora_i18n::tr!("ui-inventory-auto-image-retry")).clicked();
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
        if !current_asset.is_nil() && current_texture.is_none() {
            self.inventory_ui.wanted_images.insert(current_asset);
        }
        if cancel {
            open = false;
        }
        if let Some(s) = &mut self.inventory_auto_image {
            s.open = open;
            if s.yaw != yaw || s.zoom != zoom {
                s.yaw = yaw;
                s.zoom = zoom;
                s.zoom_default = false;
                // The next preview follows the new view right away.
                s.preview_at = None;
            }
        }
        if centre != self.settings.auto_image_centre {
            self.settings.auto_image_centre = centre;
            self.settings.save();
        }
        if background != self.settings.auto_image_background {
            self.settings.auto_image_background = background;
            self.settings.save();
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
            s.blank = false;
        }
        if let Some(yaw) = view {
            self.inventory_auto_image_view(yaw);
        }
        if reset {
            self.reset_inventory_auto_image_view();
        }
        if retry {
            self.retry_inventory_auto_image();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_picture_is_detected_as_blank() {
        let flat = [90u8, 90, 90, 255];
        let pixels: Vec<u8> = flat.repeat(16);
        assert!(looks_blank(&pixels));
        let mut varied = pixels.clone();
        varied[0] = 200;
        assert!(!looks_blank(&varied));
    }

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
        // With no dominant skinned joint, the head turned a quarter turn right
        // takes the garment's bind front (+X) with it.
        let point = Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2);
        let front = subject_front(true, Some(Quat::IDENTITY), Some(point), None);
        assert!((front - Vec3::NEG_Y).length() < 1e-5, "{front:?}");
    }

    #[test]
    fn the_joint_front_composes_with_the_avatars_facing() {
        let quarter = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let front = subject_front(true, Some(quarter), Some(quarter), None);
        assert!((front - Vec3::NEG_X).length() < 1e-5, "{front:?}");
    }

    #[test]
    fn the_alpha_centroid_follows_what_is_visible() {
        let mut pixels = vec![0u8; 4 * 4 * 4];
        // A two-pixel block on the right of the image.
        for y in 1..3 {
            for x in 2..4 {
                pixels[(y * 4 + x) * 4 + 3] = 255;
            }
        }
        let (x, y) = alpha_centroid(&pixels, 4, 4).expect("visible pixels");
        assert!((x - 0.75).abs() < 1e-5, "{x}");
        assert!((y - 0.5).abs() < 1e-5, "{y}");
        assert!(alpha_centroid(&[0u8; 16], 4, 4).is_none());
    }

    #[test]
    fn the_pan_converges_monotonically_onto_the_visible_centre() {
        // The mask is re-measured from the already-panned camera, so the
        // centroid is the remaining error. Adding the residual must close in
        // on the real offset without ever bouncing back the other way.
        let span = 1.0f32;
        let error = 0.3f32; // the subject really sits 30 % of a frame to the right
        let mut pan = (0.0f32, 0.0f32);
        for _ in 0..9 {
            let cx = 0.5 + error - pan.0 / span;
            let next = advance_pan(pan, Some((cx, 0.5)), span);
            assert!(next.0 >= pan.0, "pan moved backwards: {pan:?} -> {next:?}");
            pan = next;
        }
        // The dead zone stops a hair short of the exact centre.
        assert!((pan.0 - error * span).abs() < 0.02, "{pan:?}");
    }

    #[test]
    fn the_pan_advances_once_per_preview_not_once_per_frame() {
        // The frame loop reuses the last preview's centroid for ~30 frames;
        // re-adding the same residual each time is what made the camera fly
        // away. A fresh preview must be needed for another step.
        let (mut pan, mut dirty) = ((0.0f32, 0.0f32), true);
        for _ in 0..60 {
            if dirty {
                pan = advance_pan(pan, Some((0.8, 0.5)), 1.0);
                dirty = false;
            }
        }
        assert!((pan.0 - 0.21).abs() < 1e-5, "{pan:?}");
        // a new preview arrives: one more step, from the new residual
        pan = advance_pan(pan, Some((0.59, 0.5)), 1.0);
        assert!((pan.0 - 0.273).abs() < 1e-5, "{pan:?}");
    }

    #[test]
    fn the_pan_ignores_a_residual_inside_the_dead_zone() {
        let pan = (0.2f32, -0.1f32);
        assert_eq!(advance_pan(pan, Some((0.5 + PAN_DEAD_ZONE * 0.5, 0.5)), 1.0), pan);
        assert_eq!(advance_pan(pan, Some((0.5, 0.5 - PAN_DEAD_ZONE * 0.5)), 1.0), pan);
        // A real offset still moves it.
        assert_ne!(advance_pan(pan, Some((0.7, 0.5)), 1.0), pan);
    }

    #[test]
    fn the_pan_search_is_bounded_for_a_subject_too_large_to_fit() {
        // A whole body clipped by the frame keeps reporting an offset with no
        // stable centre. The cap ends the search so the camera holds still.
        let (mut pan, mut steps) = ((0.0f32, 0.0f32), 0u8);
        for i in 0..40 {
            if steps >= MAX_PAN_STEPS {
                break;
            }
            // An alternating edge keeps the residual outside the dead zone.
            let cx = if i % 2 == 0 { 0.8 } else { 0.2 };
            let next = advance_pan(pan, Some((cx, 0.5)), 1.0);
            if next != pan {
                steps += 1;
            }
            pan = next;
        }
        assert_eq!(steps, MAX_PAN_STEPS);
    }

    #[test]
    fn the_pan_ignores_a_missing_mask_and_stays_bounded() {
        assert_eq!(advance_pan((1.0, -2.0), None, 4.0), (1.0, -2.0));
        let pan = advance_pan((0.0, 0.0), Some((1.0, 0.0)), 4.0);
        assert!(pan.0.hypot(pan.1) <= 4.0, "{pan:?}");
        // A subject low in the frame pans the camera down (negative up).
        let pan = advance_pan((0.0, 0.0), Some((0.5, 0.9)), 4.0);
        assert!(pan.1 < 0.0, "{pan:?}");
    }

    #[test]
    fn a_body_joint_uses_the_avatars_bind_front() {
        // Shorts weighted to the pelvis face the way their wearer faces, but
        // authored along the bind frame's +X: a quarter turn from avatar +Y.
        let point = Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2);
        let front = subject_front(true, Some(Quat::IDENTITY), Some(point), Some(Vec3::NEG_Y));
        assert!((front - Vec3::X).length() < 1e-5, "{front:?}");
        let turned = subject_front(
            true,
            Some(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
            Some(point),
            Some(Vec3::NEG_Y),
        );
        assert!((turned - Vec3::Y).length() < 1e-5, "{turned:?}");
    }

    #[test]
    fn a_dominant_joint_pointing_up_falls_through_to_the_point() {
        let point = Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2);
        let front = subject_front(true, Some(Quat::IDENTITY), Some(point), Some(Vec3::Z));
        assert!((front - Vec3::NEG_Y).length() < 1e-5, "{front:?}");
    }

    #[test]
    fn an_unknown_joint_or_a_non_rigged_prim_keeps_the_avatar_forward() {
        let avatar = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let point = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let no_joint = subject_front(true, Some(avatar), None, None);
        assert!((no_joint - Vec3::Y).length() < 1e-5, "{no_joint:?}");
        let non_rigged = subject_front(false, Some(avatar), Some(point), Some(Vec3::X));
        assert!((non_rigged - Vec3::NEG_X).length() < 1e-5, "{non_rigged:?}");
    }

    #[test]
    fn a_joint_pointing_straight_up_falls_back_to_the_bind_front() {
        let point = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
        let front = subject_front(true, Some(Quat::IDENTITY), Some(point), None);
        assert!((front - Vec3::X).length() < 1e-5, "{front:?}");
    }
}
