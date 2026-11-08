//! Right-click "Save as ▸ Collada": export a linkset to a `.dae` file.
//!
//! The client gathers what it already renders: tessellated prims
//! (`aurora_prim::generate_volume`), mesh assets at the finest held level of
//! detail, sculpt maps, trees and their face textures, each placed with the
//! world transform the scene draws it with. Textures save as PNG files next
//! to the `.dae`; per-face texture-entry transforms are not baked. When the
//! picked file's directory is not writable (a sandboxed portal save), they
//! embed in the `.dae` as `data:` URIs instead.
//!
//! Mesh, sculpt and texture assets download in the background: the export
//! waits for them up to a deadline, then writes whatever is ready and
//! reports what was skipped. The simulator's permissions are irrelevant to
//! this file: anything drawn can be exported.
use super::context_menu::linkset_root;
use crate::build;
use crate::export_collada::{ExportDocument, ExportGeometry, ExportMaterial, ExportNode, ExportPart, write_collada};
use crate::scene::Scene;
use crate::world::notifications::{Data, Kind};
use aurora_prim::params::{LL_PCODE_LEGACY_GRASS, LL_PCODE_LEGACY_TREE, LL_PCODE_TREE_NEW, LL_PCODE_VOLUME};
use aurora_prim::te::{BLANK_TEXTURE, DEFAULT_TEXTURE};
use glam::{Mat3, Mat4, Quat, Vec3};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Highest prim tessellation (`DETAIL_SCALES` of `aurora_prim`).
const DETAIL: f32 = 4.0;
/// Finest mesh level of detail asked for.
const MESH_LOD: u8 = 3;
/// How long the export waits for mesh and sculpt downloads.
const FETCH_GRACE: Duration = Duration::from_secs(45);

/// "Save as ▸ Collada" in flight (at most one).
#[derive(Default)]
pub(super) struct ObjectExport {
    pending: Option<PendingExport>,
}

struct PendingExport {
    /// Full id of the linkset root clicked.
    id: Uuid,
    /// Default file stem for the save dialog.
    name: String,
    /// The save dialog runs on its own thread; the path arrives here.
    rx: Option<mpsc::Receiver<Option<PathBuf>>>,
    path: Option<PathBuf>,
    /// When the path arrived: the download wait starts then.
    fetch_start: Option<Instant>,
    /// Decoded mesh faces by asset id (None: failed, skipped without
    /// waiting again), kept while the export waits.
    mesh_cache: HashMap<Uuid, Option<Vec<aurora_assets::MeshFace>>>,
    /// Face textures wanted for the export, in first-use order.
    textures: Vec<Uuid>,
    warned_flexible: bool,
}

/// A mesh asset's state for the export.
enum MeshStatus {
    Ready(Vec<aurora_assets::MeshFace>),
    Waiting,
    Failed,
}

impl super::App {
    /// "Save as ▸ Collada" of the object menu: ask where, then gather.
    pub(super) fn begin_object_export(&mut self, id: Uuid) {
        let Some(idx) = self.world.objects.index_of_uuid(&id) else {
            return;
        };
        let root = linkset_root(&self.world, idx);
        let root_id = self.world.objects.get(root).map(|o| o.full_id).unwrap_or(id);
        let name = self
            .build
            .props
            .get(&root_id)
            .or_else(|| self.interactions.props.get(&root_id))
            .map(|props| props.name.clone())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "object".to_string());
        let stem = sanitize_stem(&name);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let path = rfd::FileDialog::new()
                .set_file_name(format!("{stem}.dae"))
                .add_filter("DAE", &["dae"])
                .save_file();
            let _ = tx.send(path);
        });
        self.object_export.pending = Some(PendingExport {
            id: root_id,
            name,
            rx: Some(rx),
            path: None,
            fetch_start: None,
            mesh_cache: HashMap::new(),
            textures: Vec::new(),
            warned_flexible: false,
        });
        log::info!("object export: gathering {root_id}");
    }

    /// Each frame: take the dialog's path, wait for downloads, write.
    pub(super) fn poll_object_export(&mut self) {
        if self.object_export.pending.is_none() {
            return;
        }
        let rx = self.object_export.pending.as_mut().and_then(|pending| pending.rx.take());
        if let Some(rx) = rx {
            match rx.try_recv() {
                Ok(Some(path)) => {
                    let pending = self.object_export.pending.as_mut().expect("export pending");
                    pending.path = Some(path);
                    pending.fetch_start = Some(Instant::now());
                }
                Ok(None) | Err(mpsc::TryRecvError::Disconnected) => {
                    self.object_export.pending = None;
                    return;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.object_export.pending.as_mut().expect("export pending").rx = Some(rx);
                    return;
                }
            }
        }
        let has_path = self.object_export.pending.as_ref().is_some_and(|pending| pending.path.is_some());
        if !has_path {
            return;
        }
        let waiting = self.object_export_gather();
        let within_grace = self
            .object_export
            .pending
            .as_ref()
            .is_some_and(|pending| pending.fetch_start.is_some_and(|start| start.elapsed() < FETCH_GRACE));
        if waiting && within_grace {
            return;
        }
        self.object_export_write();
    }

    /// Request every missing asset so they download together. True while
    /// anything is still downloading.
    fn object_export_gather(&mut self) -> bool {
        let Some(id) = self.object_export.pending.as_ref().map(|pending| pending.id) else {
            return false;
        };
        let Some(idx) = self.world.objects.index_of_uuid(&id) else {
            return false;
        };
        let root = linkset_root(&self.world, idx);
        let mut waiting = false;
        for &prim in &build::family(&self.world, root) {
            let Some(o) = self.world.objects.get(prim) else { continue };
            if o.is_avatar() || o.pcode == LL_PCODE_LEGACY_GRASS {
                continue;
            }
            if o.pcode == LL_PCODE_LEGACY_TREE || o.pcode == LL_PCODE_TREE_NEW {
                if self.object_export_want_texture(aurora_prim::tree::tree_species(o.tree_species).texture) {
                    waiting = true;
                }
                continue;
            }
            if o.pcode != LL_PCODE_VOLUME {
                continue;
            }
            let face_ids: Vec<Uuid> = o
                .te
                .as_ref()
                .map(|te| te.faces.iter().map(|face| face.texture).collect())
                .unwrap_or_default();
            let sculpt = o.visual_volume().sculpt;
            for id in face_ids {
                if self.object_export_want_texture(id) {
                    waiting = true;
                }
            }
            let Some(sculpt) = sculpt else { continue };
            if sculpt.is_mesh() {
                let Some(pending) = self.object_export.pending.as_mut() else { return false };
                if matches!(mesh_status(&mut self.scene.meshes, pending, &sculpt.texture), MeshStatus::Waiting) {
                    waiting = true;
                }
            } else if self.scene.textures.sculpt_map(&sculpt.texture).is_none() {
                self.scene.textures.want_sculpt(&sculpt.texture);
                waiting = true;
            }
        }
        waiting
    }

    /// Record a face texture and ask for its full resolution. True while it
    /// is still downloading. The default texture needs no file.
    fn object_export_want_texture(&mut self, id: Uuid) -> bool {
        if id == DEFAULT_TEXTURE {
            return false;
        }
        let ready = {
            let source = self.scene.textures.preview_source(&id);
            source.local.is_some()
                || source.procedural
                || id == BLANK_TEXTURE
                || crate::scene::textures::is_builtin_transparent(&id)
                || source.decoded == Some(0)
        };
        if ready {
            let Some(pending) = self.object_export.pending.as_mut() else {
                return false;
            };
            if !pending.textures.contains(&id) {
                pending.textures.push(id);
            }
            return false;
        }
        self.scene.textures.note_usage_fixed(&id, 16384.0);
        true
    }

    /// Build the document from what is ready and write the file.
    fn object_export_write(&mut self) {
        let Some(mut pending) = self.object_export.pending.take() else {
            return;
        };
        let Some(path) = pending.path.take() else { return };
        let id = pending.id;
        let Some(idx) = self.world.objects.index_of_uuid(&id) else {
            return;
        };
        let now = Instant::now();
        let root = linkset_root(&self.world, idx);
        let mut doc = ExportDocument {
            name: pending.name.clone(),
            ..ExportDocument::default()
        };
        let mut materials: HashMap<([u32; 4], Option<Uuid>), usize> = HashMap::new();
        let mut skipped: Vec<String> = Vec::new();
        let prims = build::family(&self.world, root);
        for (prim_no, &prim) in prims.iter().enumerate() {
            let Some(o) = self.world.objects.get(prim) else { continue };
            if o.is_avatar() {
                continue;
            }
            let Some((pos, rot, _)) = Scene::object_transform(&self.world, prim, now, 0) else {
                skipped.push(format!("prim {prim_no}: no transform"));
                continue;
            };
            let scale = o.visual_scale().max(Vec3::splat(0.001));
            let matrix = Mat4::from_scale_rotation_translation(scale, rot, pos).to_cols_array();
            let label = format!("prim-{prim_no}");
            if o.pcode == LL_PCODE_LEGACY_GRASS {
                skipped.push(format!("{label}: grass not supported"));
                continue;
            }
            if o.pcode == LL_PCODE_LEGACY_TREE || o.pcode == LL_PCODE_TREE_NEW {
                let species = aurora_prim::tree::tree_species(o.tree_species);
                let mesh = aurora_prim::tree::generate_tree(species, 3);
                let matrix = Mat4::from_scale_rotation_translation(
                    Vec3::splat(aurora_prim::tree::tree_model_scale(scale).max(0.001)),
                    rot * Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
                    pos - Vec3::Z * 0.1,
                )
                .to_cols_array();
                let material = export_material(
                    &mut doc,
                    &mut materials,
                    "tree",
                    [1.0, 1.0, 1.0, 1.0],
                    texture_file(species.texture),
                );
                push_volume_mesh(&mut doc, &label, &mesh, matrix, material);
                continue;
            }
            if o.pcode != LL_PCODE_VOLUME {
                skipped.push(format!("{label}: unsupported type"));
                continue;
            }
            let volume = o.visual_volume();
            if let Some(sculpt) = volume.sculpt {
                if sculpt.is_mesh() {
                    match mesh_status(&mut self.scene.meshes, &mut pending, &sculpt.texture) {
                        MeshStatus::Ready(faces) => {
                            let (color, texture) = o
                                .te
                                .as_ref()
                                .map(|te| (te.face(0).color, texture_file(te.face(0).texture)))
                                .unwrap_or(([1.0; 4], None));
                            let material = export_material(&mut doc, &mut materials, "mesh", color, texture);
                            push_mesh_faces(&mut doc, &label, &faces, matrix, material);
                        }
                        MeshStatus::Waiting => skipped.push(format!("{label}: mesh still downloading")),
                        MeshStatus::Failed => skipped.push(format!("{label}: mesh unavailable")),
                    }
                } else if let Some(map) = self.scene.textures.sculpt_map(&sculpt.texture) {
                    let mesh =
                        aurora_prim::sculpt::generate_sculpt(&volume, DETAIL, map.width, map.height, map.components, &map.pixels);
                    push_volume_faces(&self.world, &mut doc, &mut materials, &label, &mesh, matrix, prim);
                } else {
                    skipped.push(format!("{label}: sculpt map still downloading"));
                }
                continue;
            }
            if o.is_flexible() && !pending.warned_flexible {
                pending.warned_flexible = true;
                skipped.push("flexible prims export unflexed".to_string());
            }
            let mesh = aurora_prim::generate_volume(&volume, DETAIL);
            push_volume_faces(&self.world, &mut doc, &mut materials, &label, &mesh, matrix, prim);
        }
        // Free the decoded meshes before writing the files.
        pending.mesh_cache.clear();
        let (images, embedded) = self.object_export_write_textures(&pending, &path, &doc, &mut skipped);
        let text = write_collada(&doc, &images);
        match std::fs::write(&path, text) {
            Ok(()) => {
                log::info!(
                    "object export: saved {} nodes and {} textures to {}",
                    doc.nodes.len(),
                    images.len(),
                    path.display()
                );
                let mut body = format!(
                    "Saved {} prims and {} textures to {}.",
                    doc.nodes.len(),
                    images.len(),
                    path.display()
                );
                if embedded > 0 {
                    body.push_str(&format!(
                        " {embedded} texture(s) are embedded: the save location is sandboxed, so sibling PNG files could not be written."
                    ));
                }
                if !skipped.is_empty() {
                    body.push_str(&format!(" Skipped: {}.", skipped.join(", ")));
                }
                self.world
                    .notifications
                    .push(Kind::Info, "Object export".to_string(), body, Data::None);
            }
            Err(e) => {
                log::warn!("object export: could not save {}: {e}", path.display());
                self.world.notifications.push(
                    Kind::Alert,
                    "Object export".to_string(),
                    format!("Could not save {}: {e}", path.display()),
                    Data::None,
                );
            }
        }
    }

    /// Full-resolution PNGs of the used face textures, next to the `.dae`
    /// when the directory is writable. A sandboxed portal save (Flatpak)
    /// only grants the picked file itself, so textures that cannot be
    /// written next to it embed in the file as `data:` URIs instead.
    /// Returns the asset ids to their `init_from` values, and how many of
    /// those are embedded.
    fn object_export_write_textures(
        &mut self,
        pending: &PendingExport,
        path: &std::path::Path,
        doc: &ExportDocument,
        skipped: &mut Vec<String>,
    ) -> (HashMap<Uuid, String>, usize) {
        let used: std::collections::HashSet<Uuid> = doc.materials.iter().filter_map(|material| material.texture).collect();
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map_or_else(|| "object".to_string(), sanitize_stem);
        let mut images = HashMap::new();
        let mut embedded = 0;
        for (index, id) in pending.textures.iter().filter(|id| used.contains(id)).enumerate() {
            let pixels = self.scene.textures.preview_source(id).bake_image(*id);
            let Some(pixels) = pixels else {
                skipped.push(format!("texture {id}: still downloading"));
                continue;
            };
            let Some(png) = encode_png(&pixels.rgba, pixels.size[0], pixels.size[1]) else {
                skipped.push(format!("texture {id}: could not encode"));
                continue;
            };
            let filename = format!("{stem}_tex{index}.png");
            match std::fs::write(path.with_file_name(&filename), &png) {
                Ok(()) => {
                    images.insert(*id, filename);
                }
                Err(e) => {
                    log::info!("object export: cannot write {filename} ({e}); embedding texture {id}");
                    images.insert(*id, data_uri(&png));
                    embedded += 1;
                }
            }
        }
        (images, embedded)
    }
}

/// Decoded faces of a mesh asset, downloading it when missing.
fn mesh_status(
    meshes: &mut crate::scene::meshes::MeshStreamer,
    pending: &mut PendingExport,
    asset: &Uuid,
) -> MeshStatus {
    if let Some(cached) = pending.mesh_cache.get(asset) {
        return match cached {
            Some(faces) => MeshStatus::Ready(faces.clone()),
            None => MeshStatus::Failed,
        };
    }
    let view = meshes.view(asset);
    let section = view.as_ref().and_then(|view| view.lod_section(MESH_LOD as usize).map(|section| section.to_vec()));
    let (Some(view), Some(section)) = (view, section) else {
        meshes.want(*asset, MESH_LOD);
        return MeshStatus::Waiting;
    };
    match aurora_assets::decode_mesh_lod(&section) {
        Ok(mut faces) => {
            // Rigged meshes render in bind pose: same transform.
            if let Some(skin) = view.skin {
                let normal = Mat3::from_mat4(skin.bind_shape).inverse().transpose();
                for face in &mut faces {
                    for position in &mut face.positions {
                        *position = skin.bind_shape.transform_point3(Vec3::from_array(*position)).to_array();
                    }
                    for normal_vec in &mut face.normals {
                        *normal_vec = (normal * Vec3::from_array(*normal_vec)).normalize_or(Vec3::Z).to_array();
                    }
                }
            }
            pending.mesh_cache.insert(*asset, Some(faces.clone()));
            MeshStatus::Ready(faces)
        }
        Err(e) => {
            log::warn!("object export: mesh {asset}: {e}");
            pending.mesh_cache.insert(*asset, None);
            MeshStatus::Failed
        }
    }
}

/// The default texture carries no image: faces using it stay flat colors.
fn texture_file(id: Uuid) -> Option<Uuid> {
    (id != DEFAULT_TEXTURE).then_some(id)
}

/// One diffuse material per face color and texture, shared by equal pairs.
fn export_material(
    doc: &mut ExportDocument,
    materials: &mut HashMap<([u32; 4], Option<Uuid>), usize>,
    name: &str,
    color: [f32; 4],
    texture: Option<Uuid>,
) -> usize {
    let key = (color.map(f32::to_bits), texture);
    if let Some(&index) = materials.get(&key) {
        return index;
    }
    let index = doc.materials.len();
    doc.materials.push(ExportMaterial {
        name: format!("{name}-{index}"),
        color,
        texture,
    });
    materials.insert(key, index);
    index
}

/// Append one part per volume face, colored by the texture entry.
fn push_volume_faces(
    world: &crate::world::World,
    doc: &mut ExportDocument,
    materials: &mut HashMap<([u32; 4], Option<Uuid>), usize>,
    label: &str,
    mesh: &aurora_prim::VolumeMesh,
    matrix: [f32; 16],
    prim: usize,
) {
    let Some(o) = world.objects.get(prim) else { return };
    let mut geometry = ExportGeometry {
        name: label.to_string(),
        ..ExportGeometry::default()
    };
    for (face_no, face) in mesh.faces.iter().enumerate() {
        if face.indices.len() < 3 {
            continue;
        }
        let (color, texture) = o
            .te
            .as_ref()
            .map(|te| (te.face(face_no).color, texture_file(te.face(face_no).texture)))
            .unwrap_or(([1.0; 4], None));
        let material = *materials.entry((color.map(f32::to_bits), texture)).or_insert_with(|| {
            let index = doc.materials.len();
            doc.materials.push(ExportMaterial {
                name: format!("face-{index}"),
                color,
                texture,
            });
            index
        });
        let base = geometry.positions.len() as u32;
        geometry.positions.extend_from_slice(&face.positions);
        geometry.normals.extend_from_slice(&face.normals);
        geometry.uvs.extend_from_slice(&face.uvs);
        geometry.parts.push(ExportPart {
            indices: face.indices.iter().map(|i| base + u32::from(*i)).collect(),
            material,
        });
    }
    if geometry.parts.is_empty() {
        return;
    }
    doc.nodes.push(ExportNode {
        name: label.to_string(),
        matrix,
        geometry: doc.geometries.len(),
    });
    doc.geometries.push(geometry);
}

/// Append a tessellated tree volume: one part per face.
fn push_volume_mesh(doc: &mut ExportDocument, label: &str, mesh: &aurora_prim::VolumeMesh, matrix: [f32; 16], material: usize) {
    let mut geometry = ExportGeometry {
        name: label.to_string(),
        ..ExportGeometry::default()
    };
    for face in &mesh.faces {
        if face.indices.len() < 3 {
            continue;
        }
        let base = geometry.positions.len() as u32;
        geometry.positions.extend_from_slice(&face.positions);
        geometry.normals.extend_from_slice(&face.normals);
        geometry.uvs.extend_from_slice(&face.uvs);
        geometry.parts.push(ExportPart {
            indices: face.indices.iter().map(|i| base + u32::from(*i)).collect(),
            material,
        });
    }
    if geometry.parts.is_empty() {
        return;
    }
    doc.nodes.push(ExportNode {
        name: label.to_string(),
        matrix,
        geometry: doc.geometries.len(),
    });
    doc.geometries.push(geometry);
}

/// Append a decoded mesh asset, rigged meshes in bind pose already.
fn push_mesh_faces(
    doc: &mut ExportDocument,
    label: &str,
    faces: &[aurora_assets::MeshFace],
    matrix: [f32; 16],
    material: usize,
) {
    let mut geometry = ExportGeometry {
        name: label.to_string(),
        ..ExportGeometry::default()
    };
    for face in faces {
        if face.indices.len() < 3 || face.positions.is_empty() {
            continue;
        }
        let base = geometry.positions.len() as u32;
        geometry.positions.extend_from_slice(&face.positions);
        if face.normals.len() == face.positions.len() {
            geometry.normals.extend_from_slice(&face.normals);
        } else {
            geometry.normals.extend(std::iter::repeat_n([0.0, 0.0, 1.0], face.positions.len()));
        }
        if face.uvs.len() == face.positions.len() {
            geometry.uvs.extend_from_slice(&face.uvs);
        } else {
            geometry.uvs.extend(std::iter::repeat_n([0.0, 0.0], face.positions.len()));
        }
        geometry.parts.push(ExportPart {
            indices: face.indices.iter().map(|i| base + u32::from(*i)).collect(),
            material,
        });
    }
    if geometry.parts.is_empty() {
        return;
    }
    doc.nodes.push(ExportNode {
        name: label.to_string(),
        matrix,
        geometry: doc.geometries.len(),
    });
    doc.geometries.push(geometry);
}

/// Encode raw RGBA pixels to a PNG file body.
fn encode_png(rgba: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    use image::ImageEncoder as _;
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(rgba, width, height, image::ColorType::Rgba8.into())
        .ok()?;
    Some(png)
}

/// A PNG file body as a Collada `data:` URI, for sandboxed saves where no
/// sibling file can be written next to the `.dae`.
fn data_uri(png: &[u8]) -> String {
    use base64::Engine as _;
    format!("data:image/png;base64,{}", base64::prelude::BASE64_STANDARD.encode(png))
}

/// Keep the save dialog's default name to plain file-stem characters.
fn sanitize_stem(name: &str) -> String {
    let stem: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let stem = stem.trim_matches('_');
    if stem.is_empty() { "object".to_string() } else { stem.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoded_png_round_trips_through_a_data_uri() {
        let png = encode_png(&[255, 0, 0, 255, 0, 255, 0, 255], 2, 1).expect("png body");
        assert_eq!(&png[1..4], b"PNG", "PNG signature");
        let uri = data_uri(&png);
        assert!(uri.starts_with("data:image/png;base64,"), "data uri prefix: {uri}");
        use base64::Engine as _;
        let back = base64::prelude::BASE64_STANDARD
            .decode(uri.trim_start_matches("data:image/png;base64,"))
            .expect("decodable payload");
        assert_eq!(back, png);
    }
}
