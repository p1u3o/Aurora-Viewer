//! Export a gathered linkset document to glTF 2.0 (`.gltf` + `.bin`, or
//! `.glb` with everything embedded).
//!
//! Works on the same neutral `ExportDocument` as `export_collada`; the
//! gathering lives in `app/object_export.rs`. glTF is Y-up with a top-left
//! UV origin while the sim is Z-up with a bottom-left one: one root node
//! rotates -90° about X (no per-vertex change, normals included) and each
//! V is flipped (`v = 1 - v`) when the buffers are written. Units are
//! metres either way.
//!
//! `.gltf` writes the vertex data to a sibling `.bin` when the directory
//! allows it and embeds it as a `data:` URI otherwise; textures behave like
//! the Collada export (sibling PNGs, `data:` URIs in a sandboxed save). A
//! `.glb` needs no sibling file at all: textures ride in the binary chunk.
use crate::export_collada::{ExportDocument, ExportMaterial};
use serde_json::{Value, json};
use std::collections::HashMap;

/// glTF 2.0 component types (spec 5.24.1), buffer targets (5.24.3) and
/// sampler modes (5.25.2) in use: REPEAT wrapping, LINEAR magnification and
/// LINEAR_MIPMAP_LINEAR minification, as the Collada export writes them.
const FLOAT: i64 = 5126;
const UNSIGNED_INT: i64 = 5125;
const ARRAY_BUFFER: i64 = 34962;
const ELEMENT_ARRAY_BUFFER: i64 = 34963;
const REPEAT: i64 = 10497;
const LINEAR: i64 = 9729;
const LINEAR_MIPMAP_LINEAR: i64 = 9987;
/// Quaternion of the Z-up to Y-up rotation (-90° about X), as (x, y, z, w).
const Z_UP_TO_Y_UP: [f64; 4] = [-std::f64::consts::FRAC_1_SQRT_2, 0.0, 0.0, std::f64::consts::FRAC_1_SQRT_2];

/// Where the vertex buffer lives in a `.gltf`.
#[derive(Clone, Copy)]
pub enum BinLocation<'a> {
    /// A sibling file of this name (the standard layout).
    SiblingFile(&'a str),
    /// A base64 `data:` URI in the JSON, for a sandboxed save where no
    /// sibling file can be written.
    Embedded,
}

/// The parts of a `.gltf` export the caller writes itself.
pub struct GltfExport {
    pub json: String,
    /// The body of the sibling `.bin` file the JSON points at; empty when
    /// the buffer was embedded as a `data:` URI.
    pub bin: Vec<u8>,
}

fn sanitize(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

/// Append one bufferView holding `bytes`, keeping the buffer 4-byte
/// aligned. `target` 0 (no target) suits image data.
fn push_view(bin: &mut Vec<u8>, views: &mut Vec<Value>, bytes: &[u8], target: i64) -> usize {
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let offset = bin.len();
    bin.extend_from_slice(bytes);
    views.push(json!({"buffer": 0, "byteOffset": offset, "byteLength": bytes.len(), "target": target}));
    views.len() - 1
}

fn floats_bytes(values: impl Iterator<Item = f32>) -> Vec<u8> {
    let mut out = Vec::new();
    for value in values {
        out.extend_from_slice(&sanitize(value).to_le_bytes());
    }
    out
}

/// The document's vertex data as one little-endian buffer, with its
/// bufferViews and accessors. UV V is flipped to glTF's top-left origin;
/// stray indices are clamped to the last vertex, like the Collada writer.
/// A geometry's accessors come in groups of four (POSITION, NORMAL,
/// TEXCOORD_0, then one per part's indices), in document order.
fn build_bin(doc: &ExportDocument) -> (Vec<u8>, Vec<Value>, Vec<Value>) {
    let mut bin = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    for geometry in &doc.geometries {
        let count = geometry.positions.len();
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for position in &geometry.positions {
            for (axis, value) in position.iter().enumerate() {
                let value = sanitize(*value);
                min[axis] = min[axis].min(value);
                max[axis] = max[axis].max(value);
            }
        }
        let pos_view = push_view(
            &mut bin,
            &mut views,
            &floats_bytes(geometry.positions.iter().flat_map(|p| *p)),
            ARRAY_BUFFER,
        );
        let normal_view = push_view(
            &mut bin,
            &mut views,
            &floats_bytes(geometry.normals.iter().flat_map(|n| *n)),
            ARRAY_BUFFER,
        );
        let uv_view = push_view(
            &mut bin,
            &mut views,
            // Bottom-left sim UVs to glTF's top-left origin.
            &floats_bytes(geometry.uvs.iter().flat_map(|uv| [uv[0], 1.0 - uv[1]])),
            ARRAY_BUFFER,
        );
        // POSITION carries min and max: the spec requires them there. An
        // empty geometry has no bounds; zeros keep the JSON finite.
        let (min, max) = if count > 0 { (min, max) } else { ([0.0; 3], [0.0; 3]) };
        accessors.push(json!({
            "bufferView": pos_view, "componentType": FLOAT, "count": count,
            "type": "VEC3", "min": min.map(f64::from), "max": max.map(f64::from),
        }));
        accessors.push(json!({"bufferView": normal_view, "componentType": FLOAT, "count": count, "type": "VEC3"}));
        accessors.push(json!({"bufferView": uv_view, "componentType": FLOAT, "count": count, "type": "VEC2"}));
        for part in &geometry.parts {
            let indices: Vec<u8> = part
                .indices
                .iter()
                .flat_map(|i| ((*i as usize).min(count.saturating_sub(1)) as u32).to_le_bytes())
                .collect();
            let index_view = push_view(&mut bin, &mut views, &indices, ELEMENT_ARRAY_BUFFER);
            accessors.push(json!({
                "bufferView": index_view, "componentType": UNSIGNED_INT,
                "count": part.indices.len(), "type": "SCALAR",
            }));
        }
    }
    (bin, views, accessors)
}

/// One PBR material: the diffuse color as `baseColorFactor` (linear, like
/// the document holds it), the texture when it has an image, no metal.
/// Faces the renderer draws blended export as `BLEND`; prims are often
/// single-sided, so materials are double-sided.
fn material_json(material: &ExportMaterial, texture_index: Option<usize>) -> Value {
    let mut pbr = json!({
        "baseColorFactor": material.color.map(f64::from),
        "metallicFactor": 0.0,
        "roughnessFactor": 1.0,
    });
    if let Some(index) = texture_index {
        pbr["baseColorTexture"] = json!({"index": index});
    }
    let mut value = json!({"name": material.name, "pbrMetallicRoughness": pbr, "doubleSided": true});
    if material.color[3] < 1.0 {
        value["alphaMode"] = json!("BLEND");
    }
    value
}

/// One mesh per geometry, one primitive per material part. Accessors are
/// grouped by geometry (three attributes, then one per part), in document
/// order; the cursor walks it in the same order.
fn meshes_json(doc: &ExportDocument) -> Vec<Value> {
    let mut meshes = Vec::with_capacity(doc.geometries.len());
    let mut cursor = 0usize;
    for geometry in &doc.geometries {
        let mut primitives = Vec::new();
        for part in &geometry.parts {
            let material = part.material.min(doc.materials.len().saturating_sub(1));
            let base = cursor;
            primitives.push(json!({
                "attributes": {"POSITION": base, "NORMAL": base + 1, "TEXCOORD_0": base + 2},
                "indices": base + 3,
                "material": material,
                "mode": 4,
            }));
            cursor += 4;
        }
        cursor += 3;
        meshes.push(json!({"name": geometry.name, "primitives": primitives}));
    }
    meshes
}

fn nodes_json(doc: &ExportDocument) -> Vec<Value> {
    let mut nodes = Vec::with_capacity(doc.nodes.len() + 1);
    nodes.push(json!({
        "name": doc.name,
        "rotation": Z_UP_TO_Y_UP,
        "children": (0..doc.nodes.len()).map(|i| i + 1).collect::<Vec<_>>(),
    }));
    for node in &doc.nodes {
        nodes.push(json!({
            "name": node.name,
            "matrix": node.matrix.iter().map(|v| f64::from(sanitize(*v))).collect::<Vec<_>>(),
            "mesh": node.geometry.min(doc.geometries.len().saturating_sub(1)),
        }));
    }
    nodes
}

/// The glTF JSON over the finished parts: meshes, nodes, materials, and
/// whichever images and buffer the layout provides.
fn gltf_json(
    doc: &ExportDocument,
    views: Vec<Value>,
    accessors: Vec<Value>,
    images: Vec<Value>,
    texture_of_asset: &HashMap<uuid::Uuid, usize>,
    buffer: Value,
) -> String {
    let mut root = json!({
        "asset": {"version": "2.0", "generator": "Aurora Viewer object export"},
        "scene": 0,
        "scenes": [{"name": doc.name, "nodes": [0]}],
        "nodes": nodes_json(doc),
        "meshes": meshes_json(doc),
        "materials": doc.materials.iter().map(|m| {
            material_json(m, m.texture.and_then(|asset| texture_of_asset.get(&asset)).copied())
        }).collect::<Vec<_>>(),
        "accessors": accessors,
        "bufferViews": views,
        "buffers": [buffer],
    });
    if !images.is_empty() {
        root["images"] = json!(images);
        root["textures"] = json!(
            (0..texture_of_asset.len())
                .map(|i| json!({"sampler": 0, "source": i}))
                .collect::<Vec<_>>()
        );
        root["samplers"] = json!(
            [{"wrapS": REPEAT, "wrapT": REPEAT, "magFilter": LINEAR, "minFilter": LINEAR_MIPMAP_LINEAR}]
        );
    }
    serde_json::to_string(&root).unwrap_or_else(|_| "{}".to_string())
}

/// Texture indices in first-use order over the materials, keeping only the
/// assets `has_image` accepts (a texture without an image leaves its
/// material a flat color, like the Collada writer).
fn texture_order(doc: &ExportDocument, has_image: impl Fn(&uuid::Uuid) -> bool) -> (Vec<uuid::Uuid>, HashMap<uuid::Uuid, usize>) {
    let mut order = Vec::new();
    let mut texture_of_asset = HashMap::new();
    for material in &doc.materials {
        let Some(asset) = material.texture else { continue };
        if texture_of_asset.contains_key(&asset) || !has_image(&asset) {
            continue;
        }
        texture_of_asset.insert(asset, order.len());
        order.push(asset);
    }
    (order, texture_of_asset)
}

/// Serialize the document to a `.gltf` file's JSON. `images` maps the
/// texture assets used by the materials to their `image.uri` values (file
/// names written next to the `.gltf`, or `data:` URIs in a sandboxed save).
/// `bin_location` says whether the vertex buffer goes to the sibling `.bin`
/// named in it or embeds as a `data:` URI. The returned `bin` is the
/// sibling file's body (empty when embedded).
pub fn write_gltf(doc: &ExportDocument, images: &HashMap<uuid::Uuid, String>, bin_location: BinLocation) -> GltfExport {
    let (bin, views, accessors) = build_bin(doc);
    let (order, texture_of_asset) = texture_order(doc, |asset| images.contains_key(asset));
    let image_values: Vec<Value> = order
        .iter()
        .map(|asset| json!({"uri": images[asset], "name": format!("{asset}")}))
        .collect();
    let buffer = match bin_location {
        BinLocation::SiblingFile(name) => json!({"uri": name, "byteLength": bin.len()}),
        BinLocation::Embedded => {
            use base64::Engine as _;
            json!({
                "uri": format!("data:application/octet-stream;base64,{}", base64::prelude::BASE64_STANDARD.encode(&bin)),
                "byteLength": bin.len(),
            })
        }
    };
    let json = gltf_json(doc, views, accessors, image_values, &texture_of_asset, buffer);
    let bin = match bin_location {
        BinLocation::SiblingFile(_) => bin,
        BinLocation::Embedded => Vec::new(),
    };
    GltfExport { json, bin }
}

/// Serialize the document to a `.glb` file body: a JSON chunk (padded with
/// spaces) and a binary chunk (padded with zeros), textures embedded as
/// PNG buffer views. Nothing needs a sibling file, so a sandboxed save
/// loses nothing.
pub fn write_glb(doc: &ExportDocument, textures: &HashMap<uuid::Uuid, Vec<u8>>) -> Vec<u8> {
    let (mut bin, mut views, accessors) = build_bin(doc);
    let (order, texture_of_asset) = texture_order(doc, |asset| textures.contains_key(asset));
    // The PNGs ride in the same buffer, after the geometry.
    let image_values: Vec<Value> = order
        .iter()
        .map(|asset| {
            let view = push_view(&mut bin, &mut views, &textures[asset], 0);
            json!({"bufferView": view, "mimeType": "image/png", "name": format!("{asset}")})
        })
        .collect();
    let json = gltf_json(
        doc,
        views,
        accessors,
        image_values,
        &texture_of_asset,
        json!({"byteLength": bin.len()}),
    );

    let mut json_bytes = json.into_bytes();
    while !json_bytes.len().is_multiple_of(4) {
        json_bytes.push(b' ');
    }
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let mut out = Vec::with_capacity(12 + 8 + json_bytes.len() + 8 + bin.len());
    // An empty buffer takes no BIN chunk (and no chunk header).
    let bin_chunk = if bin.is_empty() { 0 } else { 8 + bin.len() };
    let total = (12 + 8 + json_bytes.len() + bin_chunk) as u32;
    out.extend_from_slice(&0x46546C67u32.to_le_bytes()); // "glTF"
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&total.to_le_bytes());
    out.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x4E4F534Au32.to_le_bytes()); // "JSON"
    out.append(&mut json_bytes);
    if !bin.is_empty() {
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x004E4942u32.to_le_bytes()); // "BIN"
        out.append(&mut bin);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export_collada::{ExportGeometry, ExportNode, ExportPart};

    fn quad() -> ExportDocument {
        ExportDocument {
            name: "Test".to_string(),
            materials: vec![ExportMaterial {
                name: "white".to_string(),
                color: [1.0, 1.0, 1.0, 1.0],
                texture: None,
            }],
            geometries: vec![ExportGeometry {
                name: "quad".to_string(),
                positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
                normals: vec![[0.0, 0.0, 1.0]; 4],
                uvs: vec![[0.25, 0.75], [0.0, 0.0], [1.0, 1.0], [0.5, 0.5]],
                parts: vec![ExportPart {
                    indices: vec![0, 1, 2, 0, 2, 3],
                    material: 0,
                }],
            }],
            nodes: vec![ExportNode {
                name: "root".to_string(),
                matrix: [
                    1.0, 0.0, 0.0, 0.0, //
                    0.0, 1.0, 0.0, 0.0, //
                    0.0, 0.0, 1.0, 0.0, //
                    0.0, 0.0, 0.0, 1.0,
                ],
                geometry: 0,
            }],
        }
    }

    /// Split a GLB body into its JSON chunk and binary chunk, checking the
    /// header and the total length.
    fn glb_parts(glb: &[u8]) -> (Value, Vec<u8>) {
        assert_eq!(&glb[0..4], b"glTF", "magic: {glb:02x?}");
        assert_eq!(u32::from_le_bytes(glb[4..8].try_into().expect("version")), 2);
        let total = u32::from_le_bytes(glb[8..12].try_into().expect("total")) as usize;
        assert_eq!(total, glb.len(), "total length");
        let json_len = u32::from_le_bytes(glb[12..16].try_into().expect("json length")) as usize;
        assert_eq!(&glb[16..20], b"JSON");
        let json: Value = serde_json::from_slice(&glb[20..20 + json_len]).expect("json chunk parses");
        let mut at = 20 + json_len;
        let mut bin = Vec::new();
        if at + 8 <= glb.len() {
            let bin_len = u32::from_le_bytes(glb[at..at + 4].try_into().expect("bin length")) as usize;
            assert_eq!(&glb[at + 4..at + 8], b"BIN\0");
            bin = glb[at + 8..at + 8 + bin_len].to_vec();
            at += 8 + bin_len;
        }
        assert_eq!(at, glb.len(), "no trailing bytes");
        (json, bin)
    }

    fn f32_at(bin: &[u8], offset: usize) -> f32 {
        f32::from_le_bytes(bin[offset..offset + 4].try_into().expect("in bounds"))
    }

    #[test]
    fn glb_holds_a_valid_json_chunk_and_the_geometry() {
        let (json, bin) = glb_parts(&write_glb(&quad(), &HashMap::new()));
        assert_eq!(json["asset"]["version"], "2.0");
        assert_eq!(json["scenes"][0]["nodes"][0], 0);
        // Root node: the Z-up to Y-up rotation, children below it.
        assert_eq!(json["nodes"][0]["rotation"], json!(Z_UP_TO_Y_UP));
        assert_eq!(json["nodes"][0]["children"], json!([1]));
        assert_eq!(json["nodes"][1]["mesh"], 0);
        // The four positions, little endian, in the binary chunk.
        let view = json["bufferViews"][0].as_object().expect("position view");
        let offset = view["byteOffset"].as_u64().expect("offset") as usize;
        assert_eq!(f32_at(&bin, offset), 0.0);
        assert_eq!(f32_at(&bin, offset + 12), 1.0);
        assert_eq!(json["accessors"][0]["min"], json!([0.0, 0.0, 0.0]));
        assert_eq!(json["accessors"][0]["max"], json!([1.0, 1.0, 0.0]));
        assert_eq!(json["buffers"][0]["byteLength"], json!(bin.len()));
        // Flat color material, no texture machinery.
        assert!(json.get("images").is_none() && json.get("textures").is_none());
        assert_eq!(
            json["materials"][0]["pbrMetallicRoughness"]["baseColorFactor"],
            json!([1.0, 1.0, 1.0, 1.0])
        );
        assert_eq!(json["materials"][0]["pbrMetallicRoughness"]["metallicFactor"], 0.0);
        assert_eq!(json["materials"][0]["doubleSided"], true);
    }

    #[test]
    fn uvs_flip_to_the_top_left_origin() {
        let (json, bin) = glb_parts(&write_glb(&quad(), &HashMap::new()));
        // TEXCOORD_0 is the third accessor of geometry 0.
        let accessor = &json["accessors"][2];
        assert_eq!(accessor["type"], "VEC2");
        let view = accessor["bufferView"].as_u64().expect("view") as usize;
        let offset = json["bufferViews"][view]["byteOffset"].as_u64().expect("offset") as usize;
        assert_eq!(f32_at(&bin, offset), 0.25);
        // Sim V 0.75 becomes 0.25 at the top-left origin; each pair is
        // (u, v), so the flipped V sits 4 bytes after its U.
        assert_eq!(f32_at(&bin, offset + 4), 0.25);
        assert_eq!(f32_at(&bin, offset + 8), 0.0);
        assert_eq!(f32_at(&bin, offset + 12), 1.0);
        assert_eq!(f32_at(&bin, offset + 20), 0.0);
    }

    #[test]
    fn gltf_references_a_sibling_bin_and_image_uris() {
        let asset = uuid::Uuid::from_u128(0x7E7);
        let mut doc = quad();
        doc.materials[0].texture = Some(asset);
        let images = HashMap::from([(asset, "brick.png".to_string())]);
        let export = write_gltf(&doc, &images, BinLocation::SiblingFile("test.bin"));
        let json: Value = serde_json::from_str(&export.json).expect("json parses");
        assert_eq!(json["buffers"][0]["uri"], "test.bin");
        assert_eq!(json["buffers"][0]["byteLength"], json!(export.bin.len()));
        assert_eq!(json["images"][0]["uri"], "brick.png");
        assert_eq!(json["textures"][0]["source"], 0);
        assert_eq!(json["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["index"], 0);
        assert!(json["materials"][0]["pbrMetallicRoughness"].get("baseColorFactor").is_some());
        // The sibling .bin body is returned for the caller to write.
        assert_eq!(export.bin.len() % 4, 0, "aligned");
        assert!(export.bin.len() > 48, "holds the geometry: {}", export.bin.len());
    }

    #[test]
    fn gltf_without_a_writable_bin_embeds_it_as_a_data_uri() {
        let export = write_gltf(&quad(), &HashMap::new(), BinLocation::Embedded);
        let json: Value = serde_json::from_str(&export.json).expect("json parses");
        let uri = json["buffers"][0]["uri"].as_str().expect("uri");
        assert!(uri.starts_with("data:application/octet-stream;base64,"), "data uri: {uri}");
        use base64::Engine as _;
        let bin = base64::prelude::BASE64_STANDARD
            .decode(uri.trim_start_matches("data:application/octet-stream;base64,"))
            .expect("decodable buffer");
        assert_eq!(json["buffers"][0]["byteLength"], json!(bin.len()));
        assert!(export.bin.is_empty(), "nothing to write next to the file");
    }

    #[test]
    fn glb_embeds_the_texture_png_in_the_binary_chunk() {
        let asset = uuid::Uuid::from_u128(0x7E7);
        let mut doc = quad();
        doc.materials[0].texture = Some(asset);
        let png = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
        let (json, bin) = glb_parts(&write_glb(&doc, &HashMap::from([(asset, png.clone())])));
        assert_eq!(json["images"][0]["mimeType"], "image/png");
        assert_eq!(json["images"][0]["bufferView"], 4);
        assert_eq!(json["textures"][0]["source"], 0);
        assert_eq!(json["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["index"], 0);
        assert!(bin.windows(png.len()).any(|w| w == png), "png bytes in the binary chunk");
    }

    #[test]
    fn textured_material_without_an_image_stays_a_flat_color() {
        let asset = uuid::Uuid::from_u128(0x7E7);
        let mut doc = quad();
        doc.materials[0].texture = Some(asset);
        let (json, _) = glb_parts(&write_glb(&doc, &HashMap::new()));
        assert!(json.get("images").is_none(), "no images: {json}");
        assert!(json.get("textures").is_none(), "no textures: {json}");
        assert!(json["materials"][0]["pbrMetallicRoughness"].get("baseColorTexture").is_none());
        assert_eq!(
            json["materials"][0]["pbrMetallicRoughness"]["baseColorFactor"],
            json!([1.0, 1.0, 1.0, 1.0])
        );
    }

    #[test]
    fn translucent_color_exports_as_blend() {
        let mut doc = quad();
        doc.materials[0].color = [1.0, 1.0, 1.0, 0.5];
        let (json, _) = glb_parts(&write_glb(&doc, &HashMap::new()));
        assert_eq!(json["materials"][0]["alphaMode"], "BLEND");
    }

    #[test]
    fn empty_document_stays_valid() {
        let doc = ExportDocument::default();
        let (json, bin) = glb_parts(&write_glb(&doc, &HashMap::new()));
        assert_eq!(json["meshes"], json!([]));
        assert_eq!(json["nodes"][0]["children"], json!([]));
        assert!(json.get("textures").is_none());
        assert!(bin.is_empty(), "no binary chunk: {bin:?}");
        let export = write_gltf(&ExportDocument::default(), &HashMap::new(), BinLocation::SiblingFile("e.bin"));
        let json: Value = serde_json::from_str(&export.json).expect("json parses");
        assert_eq!(json["buffers"][0]["byteLength"], 0);
    }

    #[test]
    fn non_finite_floats_become_zero() {
        let mut doc = quad();
        doc.geometries[0].positions[0] = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY];
        doc.nodes[0].matrix[0] = f32::NAN;
        let (json, bin) = glb_parts(&write_glb(&doc, &HashMap::new()));
        let text = json.to_string();
        assert!(!text.contains("NaN") && !text.contains("null"), "finite json: {text}");
        assert_eq!(f32_at(&bin, 0), 0.0, "position x sanitized");
        assert_eq!(json["nodes"][1]["matrix"][0], 0.0);
    }

    #[test]
    fn names_and_matrices_pass_through_json_escaping() {
        let mut doc = quad();
        doc.geometries[0].name = "a\"b\\c".to_string();
        doc.nodes[0].name = "tab\tname".to_string();
        let export = write_gltf(&doc, &HashMap::new(), BinLocation::SiblingFile("t.bin"));
        let json: Value = serde_json::from_str(&export.json).expect("json parses");
        assert_eq!(json["meshes"][0]["name"], "a\"b\\c");
        assert_eq!(json["nodes"][1]["name"], "tab\tname");
    }

    /// The strongest check available offline: the viewer's own glTF
    /// importer (aurora_assets::import_gltf, the one behind Import mesh)
    /// accepts the GLB and reads the geometry back with the expected
    /// values, UVs flipped, indices intact.
    #[test]
    fn the_glb_round_trips_through_the_viewer_gltf_importer() {
        use aurora_assets::import_gltf::Document;
        let glb = write_glb(&quad(), &HashMap::new());
        let doc = Document::parse(&glb, |_| Err(aurora_assets::AssetError::invalid("no external files in a glb")))
            .expect("the importer parses the exported glb");
        let primitive = &doc.json["meshes"][0]["primitives"][0];
        let positions = aurora_assets::import_mesh::gltf::attribute::<3>(
            &doc,
            primitive["attributes"]["POSITION"].as_u64().expect("position accessor") as usize,
        )
        .expect("positions");
        assert_eq!(positions[0], [0.0, 0.0, 0.0]);
        assert_eq!(positions[2], [1.0, 1.0, 0.0]);
        let uvs = aurora_assets::import_mesh::gltf::attribute::<2>(
            &doc,
            primitive["attributes"]["TEXCOORD_0"].as_u64().expect("uv accessor") as usize,
        )
        .expect("uvs");
        assert_eq!(uvs[0], [0.25, 0.25], "sim V 0.75 flipped");
        assert_eq!(uvs[2], [1.0, 0.0], "sim V 1.0 flipped");
        let materials = doc.materials().expect("materials");
        assert_eq!(materials.len(), 1);
    }
}
