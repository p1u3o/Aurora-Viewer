//! Automatic picture of an attachable inventory item (Aurora addition).
//!
//! The Legacy Viewer's Change item thumbnail takes a snapshot of whatever
//! is on screen. Here the item is worn for the picture, the isolation shows
//! that attachment alone (its wearer is only unseen, so a rigged garment
//! still has a posed skeleton and a FULL cull state), and the square photo
//! becomes the item's picture through the ordinary thumbnail upload. The
//! camera is levelled from the attachment's own down axis: a non-rigged
//! object carried at Spine has the point's rotation applied to it, a rigged
//! one is posed from the skeleton instead.
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
use aurora_render::Renderer;
use egui::Vec2;
use glam::{Quat, Vec3};
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
    item.asset_type == 6
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
    /// The item was already worn: do not take it off at the end.
    was_worn: bool,
    camera: crate::camera::Camera,
    fixed: Option<FixedView>,
    isolation: Isolation,
    wait: Option<Instant>,
    preview_at: Option<Instant>,
    preview: Option<egui::TextureHandle>,
    shot: Option<Shot>,
    shot_wanted: bool,
    shooting: bool,
    error: String,
    saved: bool,
    /// The window's first step: the item has not been worn yet.
    confirming: bool,
    open: bool,
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
        log::info!("inventory auto image: opened for {item}");
        self.inventory_auto_image = Some(Session {
            item,
            attachment: None,
            was_worn: self.world.worn_attachment_items().contains(&item),
            camera: self.camera.clone(),
            fixed: self.camera.fixed_view,
            isolation: self.world.photo_isolation.clone(),
            wait: None,
            preview_at: None,
            preview: None,
            shot: None,
            shot_wanted: false,
            shooting: false,
            error: String::new(),
            saved: false,
            confirming: true,
            open: true,
        });
        crate::ui::widgets::open_floater(&self.egui_ctx, WINDOW, &mut true);
    }

    pub(super) fn inventory_auto_image_active(&self) -> bool {
        self.inventory_auto_image.as_ref().is_some_and(|s| !s.confirming && !s.saved)
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

    pub(super) fn close_inventory_auto_image(&mut self) {
        let Some(s) = self.inventory_auto_image.take() else {
            return;
        };
        self.camera = s.camera;
        self.camera.fixed_view = s.fixed;
        let version = self.world.photo_isolation.version;
        self.world.photo_isolation = s.isolation;
        self.world.photo_isolation.version = version.wrapping_add(1);
        if !s.was_worn && !s.confirming && !self.demo {
            self.apply_appearance_action(crate::world::appearance::Action::Remove(s.item));
        }
        log::info!("inventory auto image: closed");
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

    /// The upload has answered: the window says whether it worked.
    pub(super) fn inventory_auto_image_upload_done(&mut self, error: Option<String>) {
        if let Some(s) = &mut self.inventory_auto_image {
            s.error = error.unwrap_or_default();
            s.saved = s.error.is_empty();
        }
    }

    pub(super) fn inventory_auto_image_frame(&mut self, renderer: &Renderer) {
        let Some(s) = &self.inventory_auto_image else {
            return;
        };
        if s.confirming || s.saved {
            return;
        }
        if s.attachment.is_none() {
            if let Some((index, _)) = self.world.objects.iter().find(|(_, o)| o.attachment_item_id() == Some(s.item)) {
                let root = isolate::attachment_root_of(&self.world, index);
                if let Some((_, id)) = root
                    && let Some(s) = &mut self.inventory_auto_image
                {
                    s.attachment = Some(id);
                    s.wait = None;
                }
            } else if s.wait.is_some_and(|at| at.elapsed() > DRESS_WAIT)
                && let Some(s) = &mut self.inventory_auto_image
            {
                s.error = aurora_i18n::tr!("ui-inventory-auto-image-no-object").to_owned();
                s.wait = None;
            }
            return;
        }
        let attachment = s.attachment.expect("attachment known");
        let Some(index) = self.world.objects.index_of_uuid(&attachment) else {
            return;
        };
        let rigged = crate::scene::Scene::skeleton_owner(&self.world, index).is_some();
        let now = Instant::now();
        let Some((center, object_rot, _)) = crate::scene::Scene::object_transform(&self.world, index, now, 0) else {
            return;
        };
        let scale = self.world.objects.get(index).map(|o| o.scale).unwrap_or(Vec3::ONE);
        let radius = scale.abs().max_element().max(0.05) * 0.75;
        let fov = self.camera.fov_y.max(0.1);
        let distance = (radius / (fov * 0.5).tan() * FRAMING_MARGIN).max(0.4);
        // Look at it from the side it was authored facing: the avatar's
        // forward, or the world's +Y when there is no useful pose.
        let wearer = isolate::wearer_of(&self.world, index).map(|(i, _)| i);
        let forward_flat = wearer
            .and_then(|i| crate::scene::Scene::object_transform(&self.world, i, now, 0))
            .map(|(_, q, _)| q * Vec3::Y)
            .unwrap_or(Vec3::Y);
        let forward_flat = Vec3::new(forward_flat.x, forward_flat.y, 0.0);
        let direction = if forward_flat.length_squared() > 1e-4 {
            forward_flat.normalize()
        } else {
            Vec3::Y
        };
        let eye = center + direction * distance + Vec3::Z * radius * 0.25;
        let facing = (center - eye).normalize();
        // SL prim local -Z is down. A non-rigged attachment carries the
        // attachment point's rotation; a rigged mesh is posed by the
        // skeleton and never sees that rotation.
        let wearer_rot = wearer
            .and_then(|i| crate::scene::Scene::object_transform(&self.world, i, now, 0))
            .map(|(_, q, _)| q);
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
            && s.attachment.is_some()
        {
            s.shot_wanted = true;
        }
    }

    pub(super) fn inventory_auto_image_use(&mut self) {
        let Some((item, mut shot)) = self
            .inventory_auto_image
            .as_mut()
            .and_then(|s| s.shot.take().map(|shot| (s.item, shot)))
        else {
            return;
        };
        let saved = self.item_photo_pixels(item, shot.size, std::mem::take(&mut shot.pixels));
        if let Some(s) = &mut self.inventory_auto_image {
            s.saved = saved;
            if !saved {
                s.error = aurora_i18n::tr!("app-inventory-thumbnail-an-edit-is-already-in-progress-or-the-item").to_owned();
            }
        }
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
            if let Some(texture) = shot.as_ref().or(preview.as_ref()) {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(256.0), egui::Sense::hover());
                ui.painter().image(
                    texture.id(),
                    rect,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    crate::ui::widgets::PICTURE_TINT,
                );
            } else {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(256.0), egui::Sense::hover());
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
    fn only_object_items_can_be_photographed() {
        let mut item = aurora_net::inventory::InvItem {
            asset_type: 6,
            ..Default::default()
        };
        assert!(eligible(&item));
        for other in [0, 5, 7, 13, 20, 24, 49, 57] {
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
}
