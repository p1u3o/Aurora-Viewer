//! Export a linkset to Collada 1.4.1 (`.dae`).
//!
//! Pure geometry-to-XML conversion: gathering the prims lives in
//! `app/object_export.rs`, so this module stays unit-testable without a
//! world, a scene or the network. Units are metres, Z-up
//! (`<up_axis>Z_UP</up_axis>`), matching the simulator.

/// One material slot of a geometry: the triangle range using one material.
#[derive(Debug, Clone, Default)]
pub struct ExportPart {
    /// Triangle indices into the geometry's vertex arrays.
    pub indices: Vec<u32>,
    /// Index into [`ExportDocument::materials`].
    pub material: usize,
}

/// One geometry: shared vertex arrays, one triangle list per material.
#[derive(Debug, Clone, Default)]
pub struct ExportGeometry {
    pub name: String,
    /// Positions in the node's local space (prim scale already applied by
    /// the caller when the source stores normalized positions).
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub parts: Vec<ExportPart>,
}

/// One diffuse material: a flat color, or a texture file written next to
/// the `.dae` (keyed by asset id; see the module docs of
/// `app/object_export.rs`).
#[derive(Debug, Clone)]
pub struct ExportMaterial {
    pub name: String,
    /// Linear RGBA in 0..1 (used when the material has no texture file).
    pub color: [f32; 4],
    /// Texture asset shown with this material, if any.
    pub texture: Option<uuid::Uuid>,
}

/// One placed geometry: column-major 4x4 matrix, as composed by the caller
/// from the prim's world position, rotation and scale.
#[derive(Debug, Clone)]
pub struct ExportNode {
    pub name: String,
    pub matrix: [f32; 16],
    pub geometry: usize,
}

/// Everything one `.dae` file holds.
#[derive(Debug, Clone, Default)]
pub struct ExportDocument {
    pub name: String,
    pub materials: Vec<ExportMaterial>,
    pub geometries: Vec<ExportGeometry>,
    pub nodes: Vec<ExportNode>,
}

fn sanitize(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

fn num(value: f32, out: &mut String) {
    use std::fmt::Write as _;
    let _ = write!(out, "{:?} ", sanitize(value));
}

fn floats(values: impl IntoIterator<Item = f32>, out: &mut String) {
    for value in values {
        num(value, out);
    }
    if out.ends_with(' ') {
        out.pop();
    }
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

fn id(prefix: &str, index: usize) -> String {
    format!("{prefix}{index}")
}

/// Serialize the document to Collada 1.4.1 XML. `images` maps the texture
/// assets used by [`ExportDocument::materials`] to the file names written
/// next to the `.dae`; materials whose texture has no file stay flat colors.
pub fn write_collada(doc: &ExportDocument, images: &std::collections::HashMap<uuid::Uuid, String>) -> String {
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    out.push_str("<COLLADA xmlns=\"http://www.collada.org/2005/11/COLLADASchema\" version=\"1.4.1\">\n");
    out.push_str("  <asset>\n");
    out.push_str("    <contributor><authoring_tool>Aurora Viewer object export</authoring_tool></contributor>\n");
    out.push_str("    <unit name=\"meter\" meter=\"1\"/>\n");
    out.push_str("    <up_axis>Z_UP</up_axis>\n");
    out.push_str("  </asset>\n");

    let mut image_ids: std::collections::HashMap<uuid::Uuid, usize> = std::collections::HashMap::new();
    if images.values().any(|file| !file.is_empty()) {
        out.push_str("  <library_images>\n");
        let mut files: Vec<(&uuid::Uuid, &String)> = images.iter().filter(|(_, file)| !file.is_empty()).collect();
        files.sort_by(|a, b| a.1.cmp(b.1));
        for (asset, file) in files {
            let image = id("image", image_ids.len());
            out.push_str(&format!("    <image id=\"{image}\" name=\"{asset}\">\n"));
            out.push_str(&format!("      <init_from>{}</init_from>\n", escape(file)));
            out.push_str("    </image>\n");
            image_ids.insert(*asset, image_ids.len());
        }
        out.push_str("  </library_images>\n");
    }

    out.push_str("  <library_effects>\n");
    for (index, material) in doc.materials.iter().enumerate() {
        let effect = id("effect", index);
        let image = material.texture.and_then(|asset| image_ids.get(&asset).copied());
        out.push_str(&format!("    <effect id=\"{effect}\">\n"));
        out.push_str("      <profile_COMMON>\n");
        if let Some(image) = image {
            let (surface, sampler) = (id("surface", index), id("sampler", index));
            let image_id = id("image", image);
            out.push_str(&format!("        <newparam sid=\"{surface}\">\n"));
            out.push_str(&format!("          <surface type=\"2D\">\n            <init_from>{image_id}</init_from>\n            <format>A8R8G8B8</format>\n          </surface>\n"));
            out.push_str("        </newparam>\n");
            out.push_str(&format!("        <newparam sid=\"{sampler}\">\n"));
            out.push_str(&format!(
                "          <sampler2D>\n            <source>{surface}</source>\n            <minfilter>LINEAR_MIPMAP_LINEAR</minfilter>\n            <magfilter>LINEAR</magfilter>\n            <wrap_s>REPEAT</wrap_s>\n            <wrap_t>REPEAT</wrap_t>\n          </sampler2D>\n"
            ));
            out.push_str("        </newparam>\n");
        }
        out.push_str("        <technique sid=\"common\">\n");
        out.push_str("          <lambert>\n");
        if image.is_some() {
            let sampler = id("sampler", index);
            out.push_str(&format!("            <diffuse><texture texture=\"{sampler}\" texcoord=\"UV0\"/></diffuse>\n"));
        } else {
            out.push_str("            <diffuse><color>");
            floats(material.color, &mut out);
            out.push_str("</color></diffuse>\n");
        }
        out.push_str("          </lambert>\n");
        out.push_str("        </technique>\n");
        out.push_str("      </profile_COMMON>\n");
        out.push_str("    </effect>\n");
    }
    out.push_str("  </library_effects>\n");

    out.push_str("  <library_materials>\n");
    for (index, material) in doc.materials.iter().enumerate() {
        let (material_id, effect) = (id("material", index), id("effect", index));
        out.push_str(&format!(
            "    <material id=\"{material_id}\" name=\"{}\">\n",
            escape(&material.name)
        ));
        out.push_str(&format!("      <instance_effect url=\"#{effect}\"/>\n"));
        out.push_str("    </material>\n");
    }
    out.push_str("  </library_materials>\n");

    out.push_str("  <library_geometries>\n");
    for (index, geometry) in doc.geometries.iter().enumerate() {
        let geometry_id = id("geometry", index);
        let vertex_count = geometry.positions.len();
        out.push_str(&format!(
            "    <geometry id=\"{geometry_id}\" name=\"{}\">\n",
            escape(&geometry.name)
        ));
        out.push_str("      <mesh>\n");
        write_source(&mut out, &format!("{geometry_id}-positions"), "XYZ", &geometry.positions);
        write_source(&mut out, &format!("{geometry_id}-normals"), "XYZ", &geometry.normals);
        write_uv_source(&mut out, &format!("{geometry_id}-uvs"), &geometry.uvs);
        out.push_str(&format!(
            "        <vertices id=\"{geometry_id}-vertices\">\n          <input semantic=\"POSITION\" source=\"#{geometry_id}-positions\"/>\n        </vertices>\n"
        ));
        for part in &geometry.parts {
            let material_id = id("material", part.material.min(doc.materials.len().saturating_sub(1)));
            out.push_str(&format!(
                "        <triangles material=\"{material_id}\" count=\"{}\">\n",
                part.indices.len() / 3
            ));
            out.push_str(&format!(
                "          <input semantic=\"VERTEX\" source=\"#{geometry_id}-vertices\" offset=\"0\"/>\n"
            ));
            out.push_str(&format!(
                "          <input semantic=\"NORMAL\" source=\"#{geometry_id}-normals\" offset=\"0\"/>\n"
            ));
            out.push_str(&format!("          <input semantic=\"TEXCOORD\" source=\"#{geometry_id}-uvs\" offset=\"0\" set=\"0\"/>\n"));
            out.push_str("          <p>");
            for (i, vertex) in part.indices.iter().enumerate() {
                // Clamp stray indices to the last vertex rather than
                // writing a dangling reference.
                let vertex = (*vertex as usize).min(vertex_count.saturating_sub(1));
                if i > 0 {
                    out.push(' ');
                }
                use std::fmt::Write as _;
                let _ = write!(out, "{vertex}");
            }
            out.push_str("</p>\n");
            out.push_str("        </triangles>\n");
        }
        out.push_str("      </mesh>\n");
        out.push_str("    </geometry>\n");
    }
    out.push_str("  </library_geometries>\n");

    out.push_str("  <library_visual_scenes>\n");
    out.push_str("    <visual_scene id=\"scene\" name=\"");
    out.push_str(&escape(&doc.name));
    out.push_str("\">\n");
    for (index, node) in doc.nodes.iter().enumerate() {
        let geometry_id = id("geometry", node.geometry.min(doc.geometries.len().saturating_sub(1)));
        out.push_str(&format!(
            "      <node id=\"node{index}\" name=\"{}\" type=\"NODE\">\n",
            escape(&node.name)
        ));
        out.push_str("        <matrix>");
        floats(node.matrix, &mut out);
        out.push_str("</matrix>\n");
        out.push_str(&format!("        <instance_geometry url=\"#{geometry_id}\">\n"));
        let mut bound: Vec<usize> = doc
            .geometries
            .get(node.geometry)
            .map(|geometry| geometry.parts.iter().map(|part| part.material).collect())
            .unwrap_or_default();
        bound.sort_unstable();
        bound.dedup();
        if !bound.is_empty() {
            out.push_str("          <bind_material>\n            <technique_common>\n");
            for material in bound {
                let material_id = id("material", material.min(doc.materials.len().saturating_sub(1)));
                out.push_str(&format!("              <instance_material symbol=\"{material_id}\" target=\"#{material_id}\">\n"));
                out.push_str(
                    "                <bind_vertex_input semantic=\"TEXCOORD\" input_semantic=\"TEXCOORD\" input_set=\"0\"/>\n",
                );
                out.push_str("              </instance_material>\n");
            }
            out.push_str("            </technique_common>\n          </bind_material>\n");
        }
        out.push_str("        </instance_geometry>\n");
        out.push_str("      </node>\n");
    }
    out.push_str("    </visual_scene>\n");
    out.push_str("  </library_visual_scenes>\n");
    out.push_str("  <scene><instance_visual_scene url=\"#scene\"/></scene>\n");
    out.push_str("</COLLADA>\n");
    out
}

fn write_source(out: &mut String, id: &str, params: &str, triples: &[[f32; 3]]) -> usize {
    out.push_str(&format!("        <source id=\"{id}\">\n"));
    out.push_str(&format!("          <float_array id=\"{id}-array\" count=\"{}\">", triples.len() * 3));
    for triple in triples {
        floats(*triple, out);
        out.push(' ');
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out.push_str("</float_array>\n");
    out.push_str("          <technique_common>\n");
    out.push_str(&format!("            <accessor source=\"#{id}-array\" count=\"{}\" stride=\"3\">\n", triples.len()));
    for param in params.chars() {
        use std::fmt::Write as _;
        let _ = writeln!(out, "              <param name=\"{param}\" type=\"float\"/>");
    }
    out.push_str("            </accessor>\n");
    out.push_str("          </technique_common>\n");
    out.push_str("        </source>\n");
    triples.len()
}

fn write_uv_source(out: &mut String, id: &str, uvs: &[[f32; 2]]) {
    out.push_str(&format!("        <source id=\"{id}\">\n"));
    out.push_str(&format!("          <float_array id=\"{id}-array\" count=\"{}\">", uvs.len() * 2));
    for uv in uvs {
        floats(*uv, out);
        out.push(' ');
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out.push_str("</float_array>\n");
    out.push_str("          <technique_common>\n");
    out.push_str(&format!("            <accessor source=\"#{id}-array\" count=\"{}\" stride=\"2\">\n", uvs.len()));
    out.push_str("              <param name=\"S\" type=\"float\"/>\n");
    out.push_str("              <param name=\"T\" type=\"float\"/>\n");
    out.push_str("            </accessor>\n");
    out.push_str("          </technique_common>\n");
    out.push_str("        </source>\n");
}

#[cfg(test)]
mod tests {
    use super::*;

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
                uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
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

    #[test]
    fn quad_writes_one_geometry_with_two_triangles() {
        let xml = write_collada(&quad(), &std::collections::HashMap::new());
        assert!(xml.contains("<up_axis>Z_UP</up_axis>"), "Z-up marker:\n{xml}");
        assert!(xml.contains("<triangles material=\"material0\" count=\"2\">"), "triangle list:\n{xml}");
        assert!(xml.contains("<p>0 1 2 0 2 3</p>"), "index list:\n{xml}");
        assert!(xml.contains("<float_array id=\"geometry0-positions-array\" count=\"12\">"), "positions:\n{xml}");
        assert!(xml.contains("instance_geometry url=\"#geometry0\""), "instance:\n{xml}");
        assert!(xml.contains("<diffuse><color>1.0 1.0 1.0 1.0</color></diffuse>"), "material:\n{xml}");
        assert!(!xml.contains("<library_images>"), "no images:\n{xml}");
    }

    #[test]
    fn textured_material_references_its_image_file() {
        let asset = uuid::Uuid::from_u128(0x7E7);
        let mut doc = quad();
        doc.materials[0].texture = Some(asset);
        let xml = write_collada(&doc, &std::collections::HashMap::from([(asset, "brick.png".to_string())]));
        assert!(xml.contains("<init_from>brick.png</init_from>"), "image:\n{xml}");
        assert!(
            xml.contains("<texture texture=\"sampler0\" texcoord=\"UV0\"/>"),
            "sampler:\n{xml}"
        );
        assert!(!xml.contains("<diffuse><color>"), "no flat color:\n{xml}");
    }

    #[test]
    fn embedded_data_uri_passes_through_to_init_from() {
        let uri = "data:image/png;base64,iVBORw0KGgo=".to_string();
        let mut doc = quad();
        doc.materials[0].texture = Some(uuid::Uuid::from_u128(0x7E7));
        let xml = write_collada(&doc, &std::collections::HashMap::from([(uuid::Uuid::from_u128(0x7E7), uri.clone())]));
        assert!(xml.contains(&format!("<init_from>{uri}</init_from>")), "data uri:\n{xml}");
    }

    #[test]
    fn textured_material_without_a_file_stays_a_flat_color() {
        let mut doc = quad();
        doc.materials[0].texture = Some(uuid::Uuid::from_u128(0x7E7));
        let xml = write_collada(&doc, &std::collections::HashMap::new());
        assert!(!xml.contains("<library_images>"), "no images:\n{xml}");
        assert!(xml.contains("<diffuse><color>1.0 1.0 1.0 1.0</color></diffuse>"), "color:\n{xml}");
    }

    #[test]
    fn names_are_escaped() {
        let mut doc = quad();
        doc.geometries[0].name = "a<b>&\"'c".to_string();
        let xml = write_collada(&doc, &std::collections::HashMap::new());
        assert!(xml.contains("a&lt;b&gt;&amp;&quot;&apos;c"), "escaped:\n{xml}");
        assert!(!xml.contains("a<b>"), "raw:\n{xml}");
    }

    #[test]
    fn empty_document_stays_valid() {
        let xml = write_collada(
            &ExportDocument {
                name: "empty".to_string(),
                ..ExportDocument::default()
            },
            &std::collections::HashMap::new(),
        );
        assert!(xml.contains("<library_geometries>\n  </library_geometries>"), "geometries:\n{xml}");
        assert!(xml.ends_with("</COLLADA>\n"), "root close:\n{xml}");
    }

    #[test]
    fn non_finite_floats_become_zero() {
        let mut doc = quad();
        doc.geometries[0].positions[0] = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY];
        let xml = write_collada(&doc, &std::collections::HashMap::new());
        assert!(!xml.contains("NaN") && !xml.contains("inf"), "finite:\n{xml}");
    }
}
