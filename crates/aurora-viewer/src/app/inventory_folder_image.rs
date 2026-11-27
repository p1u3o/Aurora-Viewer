//! The folder counterpart of the item's auto image: a square picture made
//! from the pictures of the folder's own entries. This needs no wear and no
//! rendering — the tiles are the images the inventory already shows.

use super::App;
use aurora_render::Renderer;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// How long a child picture may take to decode before the folder picture is
/// made from whatever arrived.
const WAIT: Duration = Duration::from_secs(20);
const SIDE: u32 = 256;
const CELL: u32 = 124;
const GAP: u32 = 4;

pub(super) struct FolderJob {
    folder: Uuid,
    assets: Vec<Uuid>,
    images: HashMap<Uuid, (u32, u32, Vec<u8>)>,
    at: Instant,
}

impl App {
    /// Make the folder's picture from up to four of its children's pictures.
    pub(super) fn folder_auto_image(&mut self, folder: Uuid) {
        if !crate::world::inventory::actions::mutable(&self.world.inventory, folder) {
            return;
        }
        if self.inventory_images.folder.is_some() || self.inventory_images.auto_image.is_some() {
            return;
        }
        let assets = folder_pictures(&self.world.inventory, folder);
        if assets.is_empty() {
            self.inventory_ui.message = aurora_i18n::tr!("ui-inventory-auto-image-folder-empty").to_owned();
            return;
        }
        log::info!("inventory folder image: composing {} child picture(s) for {folder}", assets.len());
        self.inventory_images.folder = Some(FolderJob {
            folder,
            assets,
            images: HashMap::new(),
            at: Instant::now(),
        });
    }

    /// Take the child images the texture streamer has decoded, if any, and
    /// finish the picture once all (or enough) are here.
    pub(super) fn folder_image_frame(&mut self, renderer: &mut Renderer) {
        if self.inventory_images.folder.is_none() {
            return;
        }
        let assets = self.inventory_images.folder.as_ref().expect("folder job").assets.clone();
        for asset in assets {
            if self
                .inventory_images
                .folder
                .as_ref()
                .is_some_and(|job| job.images.contains_key(&asset))
            {
                continue;
            }
            self.scene.textures.want_ui_image(renderer, asset, &self.scene.jobs);
            if let Some(image) = self.scene.textures.take_ui_image(&asset)
                && let Some(job) = &mut self.inventory_images.folder
            {
                job.images.insert(asset, image);
            }
        }
        let ready = self
            .inventory_images
            .folder
            .as_ref()
            .is_some_and(|job| job.images.len() == job.assets.len() || job.at.elapsed() > WAIT);
        if !ready {
            return;
        }
        let Some(job) = self.inventory_images.folder.take() else {
            return;
        };
        let order = job.assets.iter().filter_map(|id| job.images.get(id)).cloned().collect::<Vec<_>>();
        let pixels = collage(&order);
        let folder = job.folder;
        if let Some(request) = self.item_photo_pixels(folder, (SIDE, SIDE), pixels) {
            self.inventory_images.folder_upload = Some(request);
        } else {
            self.inventory_ui.message = aurora_i18n::tr!("app-inventory-thumbnail-an-edit-is-already-in-progress-or-the-item").to_owned();
        }
    }
}

/// A folder's own entries' pictures, then their textures, in folder order.
/// Never walks a subfolder: the same rule the gallery's folder tiles use.
pub(super) fn folder_pictures(inv: &crate::world::inventory::Inventory, folder: Uuid) -> Vec<Uuid> {
    let Some(entry) = inv.folders.get(&folder) else {
        return Vec::new();
    };
    // RLVa can hide textures: with no texture preview, a collage of the
    // children's pictures would be misleading (`can_preview` gates asset
    // type 0, textures, against the behaviour's preview mask).
    if !inv.can_preview(0) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for id in entry.items.iter().take(256) {
        let item = inv.items.get(id);
        let picture = item
            .map(|it| it.thumbnail)
            .filter(|id| !id.is_nil())
            .or_else(|| item.filter(|it| it.asset_type == 0 && !it.asset_id.is_nil()).map(|it| it.asset_id));
        if let Some(picture) = picture
            && !out.contains(&picture)
        {
            out.push(picture);
        }
        if out.len() == 4 {
            break;
        }
    }
    out
}

/// The pictures in a square, two by two, over a dark field. Each child is
/// centre-cropped so the result does not stretch a landscape snapshot.
fn collage(images: &[(u32, u32, Vec<u8>)]) -> Vec<u8> {
    let mut canvas = image::RgbaImage::from_pixel(SIDE, SIDE, image::Rgba([0x20, 0x23, 0x2b, 255]));
    for (i, (w, h, rgba)) in images.iter().take(4).enumerate() {
        let (w, h) = (*w, *h);
        let Some(source) = image::RgbaImage::from_raw(w, h, rgba.clone()) else {
            continue;
        };
        let edge = w.min(h);
        if edge == 0 {
            continue;
        }
        let cropped = image::imageops::crop_imm(&source, (w - edge) / 2, (h - edge) / 2, edge, edge).to_image();
        let tile = image::imageops::resize(&cropped, CELL, CELL, image::imageops::FilterType::Lanczos3);
        let x = GAP + (i % 2) as u32 * (CELL + GAP);
        let y = GAP + (i / 2) as u32 * (CELL + GAP);
        image::imageops::overlay(&mut canvas, &tile, x as i64, y as i64);
    }
    canvas.into_raw()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_collage_is_a_square_with_one_tile_per_child() {
        let red = (2, 1, vec![255, 0, 0, 255, 255, 0, 0, 255]);
        let blue = (1, 2, vec![0, 0, 255, 255, 0, 0, 255, 255]);
        let pixels = collage(&[red, blue]);
        let image = image::RgbaImage::from_raw(SIDE, SIDE, pixels).expect("square");
        assert_eq!(image.get_pixel(20, 20).0, [255, 0, 0, 255]);
        assert_eq!(image.get_pixel(200, 20).0, [0, 0, 255, 255]);
        assert_eq!(image.get_pixel(20, 200).0, [0x20, 0x23, 0x2b, 255]);
    }

    #[test]
    fn an_empty_collage_still_has_the_fields_size() {
        assert_eq!(collage(&[]).len(), (SIDE * SIDE * 4) as usize);
    }
}
