//! Kaydara FBX binary writer (version 7400) for the terrain grid mesh — our own, dependency-free.
//!
//! FBX is a proprietary Autodesk format without a public specification; the binary layout
//! below is the one every clean-room reader agrees on (Blender's importer, ufbx, Optima):
//! a 27-byte header, a tree of node records (`u32` end offset, `u32` property count, `u32`
//! property bytes, `u8` name length, name, typed properties, children, 13-byte null record
//! when there are children), a null record after the top level, and a footer whose id pairs
//! with `FileId` + `CreationTime` (the FBX SDK rejects a file where the three disagree, so a
//! known-consistent triple is written). Arrays are raw (encoding 0), no compression.
//!
//! Content: one `Geometry` (quads over the grid, per-vertex normals and UVs), one `Model`, one
//! Phong `Material`; with `texture`, the baked albedo PNG rides along as an embedded
//! `Video`/`Texture` pair wired to `DiffuseColor`. Metres: `UnitScaleFactor` 100 declares the
//! unit as 100 cm, so importers show the terrain at its real size. Axes: +Y up, +Z front
//! (the mesh frame of the other exporters).
//!
//! ponytail: one mesh, one material, quads only, no animation/skinning/cameras — the FBX
//! surface Jenga Dunia produces. Verified by importing into Blender 5.2 headless and Optima.

use std::fs;
use std::path::Path;

use jd_types::JdResult;

use crate::Heightfield;
use crate::mesh::{GridMesh, MeshExportReport, bake_albedo};

/// Header magic, version 7400 follows as little-endian `u32`.
const MAGIC: &[u8] = b"Kaydara FBX Binary  \x00\x1a\x00";
const VERSION: u32 = 7400;
/// `FileId`, `CreationTime` and the footer id form a consistent set (the one Blender's
/// exporter writes); the SDK derives the footer from the first two and rejects mismatches.
const FILE_ID: [u8; 16] = [
    0x28, 0xb3, 0x2a, 0xeb, 0xb6, 0x24, 0xcc, 0xc2, 0xbf, 0xc8, 0xb0, 0x2a, 0xa9, 0x2b, 0xfc, 0xf1,
];
const CREATION_TIME: &str = "1970-01-01 10:00:00:000";
const FOOTER_ID: [u8; 16] = [
    0xfa, 0xbc, 0xab, 0x09, 0xd0, 0xc8, 0xd4, 0x66, 0xb1, 0x76, 0xfb, 0x83, 0x1c, 0xf7, 0x26, 0x7e,
];
const FOOTER_MAGIC: [u8; 16] = [
    0xf8, 0x5a, 0x8c, 0x6a, 0xde, 0xf5, 0xd9, 0x7e, 0xec, 0xe9, 0x0c, 0xe3, 0x75, 0x8f, 0x29, 0x0b,
];
/// Object ids: any distinct non-zero `i64`s; 0 is the scene root.
const ID_GEOMETRY: i64 = 1_000_001;
const ID_MODEL: i64 = 1_000_002;
const ID_MATERIAL: i64 = 1_000_003;
const ID_TEXTURE: i64 = 1_000_004;
const ID_VIDEO: i64 = 1_000_005;

/// A typed FBX property.
enum P {
    I32(i32),
    I64(i64),
    F64(f64),
    Bool(bool),
    Str(String),
    Raw(Vec<u8>),
    ArrF64(Vec<f64>),
    ArrI32(Vec<i32>),
}

impl P {
    fn s(v: &str) -> P {
        P::Str(v.to_owned())
    }
    fn write(&self, out: &mut Vec<u8>) {
        match self {
            P::I32(v) => {
                out.push(b'I');
                out.extend_from_slice(&v.to_le_bytes());
            }
            P::I64(v) => {
                out.push(b'L');
                out.extend_from_slice(&v.to_le_bytes());
            }
            P::F64(v) => {
                out.push(b'D');
                out.extend_from_slice(&v.to_le_bytes());
            }
            P::Bool(v) => {
                out.push(b'C');
                out.push(u8::from(*v));
            }
            P::Str(v) => {
                out.push(b'S');
                out.extend_from_slice(&(v.len() as u32).to_le_bytes());
                out.extend_from_slice(v.as_bytes());
            }
            P::Raw(v) => {
                out.push(b'R');
                out.extend_from_slice(&(v.len() as u32).to_le_bytes());
                out.extend_from_slice(v);
            }
            P::ArrF64(v) => {
                out.push(b'd');
                array_header(out, v.len(), 8);
                for x in v {
                    out.extend_from_slice(&x.to_le_bytes());
                }
            }
            P::ArrI32(v) => {
                out.push(b'i');
                array_header(out, v.len(), 4);
                for x in v {
                    out.extend_from_slice(&x.to_le_bytes());
                }
            }
        }
    }
}

/// Array properties: element count, encoding (0 = raw), byte length.
fn array_header(out: &mut Vec<u8>, len: usize, elem: usize) {
    out.extend_from_slice(&(len as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&((len * elem) as u32).to_le_bytes());
}

/// A node record: name, properties, children.
struct N {
    name: &'static str,
    props: Vec<P>,
    kids: Vec<N>,
}

impl N {
    fn leaf(name: &'static str, props: Vec<P>) -> N {
        N {
            name,
            props,
            kids: Vec::new(),
        }
    }
    fn tree(name: &'static str, props: Vec<P>, kids: Vec<N>) -> N {
        N { name, props, kids }
    }
    fn write(&self, out: &mut Vec<u8>) {
        let start = out.len();
        out.extend_from_slice(&[0u8; 12]); // end offset, property count, property bytes: patched below
        out.push(self.name.len() as u8);
        out.extend_from_slice(self.name.as_bytes());
        let props_start = out.len();
        for p in &self.props {
            p.write(out);
        }
        let props_len = (out.len() - props_start) as u32;
        if !self.kids.is_empty() {
            for k in &self.kids {
                k.write(out);
            }
            out.extend_from_slice(&[0u8; 13]);
        }
        let end = out.len() as u32;
        out[start..start + 4].copy_from_slice(&end.to_le_bytes());
        out[start + 4..start + 8].copy_from_slice(&(self.props.len() as u32).to_le_bytes());
        out[start + 8..start + 12].copy_from_slice(&props_len.to_le_bytes());
    }
}

/// `Properties70` entry: name, type, label, flags, value(s).
fn p70(name: &str, ty: &str, label: &str, values: Vec<P>) -> N {
    let mut props = vec![P::s(name), P::s(ty), P::s(label), P::s("")];
    props.extend(values);
    N::leaf("P", props)
}
fn p70_int(name: &str, v: i32) -> N {
    p70(name, "int", "Integer", vec![P::I32(v)])
}
fn p70_f64(name: &str, v: f64) -> N {
    p70(name, "double", "Number", vec![P::F64(v)])
}
fn p70_color(name: &str, rgb: [f64; 3]) -> N {
    p70(
        name,
        "ColorRGB",
        "Color",
        rgb.iter().map(|&c| P::F64(c)).collect(),
    )
}
fn p70_vec(name: &str, label: &str, v: [f64; 3]) -> N {
    let mut props = vec![P::s(name), P::s(label), P::s(""), P::s("A")];
    props.extend(v.iter().map(|&c| P::F64(c)));
    N::leaf("P", props)
}

fn object_type(name: &str, count: i32, template: Option<&str>) -> N {
    let mut kids = vec![N::leaf("Count", vec![P::I32(count)])];
    if let Some(t) = template {
        kids.push(N::tree(
            "PropertyTemplate",
            vec![P::s(t)],
            vec![N::leaf("Properties70", vec![])],
        ));
    }
    N::tree("ObjectType", vec![P::s(name)], kids)
}

fn connection_oo(src: i64, dst: i64) -> N {
    N::leaf("C", vec![P::s("OO"), P::I64(src), P::I64(dst)])
}
fn connection_op(src: i64, dst: i64, property: &str) -> N {
    N::leaf(
        "C",
        vec![P::s("OP"), P::I64(src), P::I64(dst), P::s(property)],
    )
}

/// Object name as FBX stores it: `name␀☺Class`.
fn object_name(name: &str, class: &str) -> String {
    format!("{name}\x00\x01{class}")
}

fn layer_element(kind: &'static str, data_name: &'static str, name: &str, data: Vec<f64>) -> N {
    N::tree(
        kind,
        vec![P::I32(0)],
        vec![
            N::leaf("Version", vec![P::I32(101)]),
            N::leaf("Name", vec![P::s(name)]),
            N::leaf("MappingInformationType", vec![P::s("ByVertice")]),
            N::leaf("ReferenceInformationType", vec![P::s("Direct")]),
            N::leaf(data_name, vec![P::ArrF64(data)]),
        ],
    )
}

/// Write the field as a binary FBX 7.4 quad mesh (metres, +Y up, world-centred). `texture`
/// embeds the baked hypsometric albedo as the material's diffuse texture.
pub fn export_fbx(
    hf: &Heightfield,
    extent_m: [f64; 2],
    path: &Path,
    step: u32,
    texture: bool,
) -> JdResult<MeshExportReport> {
    let mesh = GridMesh::build(hf, extent_m, step)?;
    let [gw, gh] = mesh.grid;
    let creator = format!("jenga-dunia {}", env!("CARGO_PKG_VERSION"));

    // Quads a→b→d→c are counter-clockwise from +Y (row c is north of row a, at −Z); the last
    // corner of each polygon is stored as `-(index + 1)`.
    let mut polygons: Vec<i32> = Vec::with_capacity(((gw - 1) * (gh - 1) * 4) as usize);
    let mut corners: Vec<i32> = Vec::with_capacity(polygons.capacity());
    for gz in 0..gh - 1 {
        for gx in 0..gw - 1 {
            let a = (gz * gw + gx) as i32;
            let (b, c, d) = (a + 1, a + gw as i32, a + gw as i32 + 1);
            polygons.extend_from_slice(&[a, b, d, -(c + 1)]);
            corners.extend_from_slice(&[a, b, d, c]);
        }
    }
    let n_vertices = gw * gh;
    let n_quads = (gw - 1) * (gh - 1);

    let mut geometry_kids = vec![
        N::leaf("GeometryVersion", vec![P::I32(124)]),
        N::leaf(
            "Vertices",
            vec![P::ArrF64(
                mesh.positions.iter().map(|&v| f64::from(v)).collect(),
            )],
        ),
        N::leaf("PolygonVertexIndex", vec![P::ArrI32(polygons)]),
        layer_element(
            "LayerElementNormal",
            "Normals",
            "",
            mesh.normals.iter().map(|&v| f64::from(v)).collect(),
        ),
        N::tree(
            "LayerElementUV",
            vec![P::I32(0)],
            vec![
                N::leaf("Version", vec![P::I32(101)]),
                N::leaf("Name", vec![P::s("map1")]),
                // ByPolygonVertex + IndexToDirect is the form the FBX SDK and Blender write and
                // the only one some readers (Optima) accept for UVs; ByVertice is kept for normals.
                N::leaf("MappingInformationType", vec![P::s("ByPolygonVertex")]),
                N::leaf("ReferenceInformationType", vec![P::s("IndexToDirect")]),
                N::leaf(
                    "UV",
                    vec![P::ArrF64(mesh.uvs.iter().map(|&v| f64::from(v)).collect())],
                ),
                N::leaf("UVIndex", vec![P::ArrI32(corners)]),
            ],
        ),
        N::tree(
            "LayerElementMaterial",
            vec![P::I32(0)],
            vec![
                N::leaf("Version", vec![P::I32(101)]),
                N::leaf("Name", vec![P::s("")]),
                N::leaf("MappingInformationType", vec![P::s("AllSame")]),
                N::leaf("ReferenceInformationType", vec![P::s("IndexToDirect")]),
                N::leaf("Materials", vec![P::ArrI32(vec![0])]),
            ],
        ),
    ];
    let layer_refs: Vec<N> = [
        "LayerElementNormal",
        "LayerElementUV",
        "LayerElementMaterial",
    ]
    .into_iter()
    .map(|kind| {
        N::tree(
            "LayerElement",
            vec![],
            vec![
                N::leaf("Type", vec![P::s(kind)]),
                N::leaf("TypedIndex", vec![P::I32(0)]),
            ],
        )
    })
    .collect();
    let mut layer_kids = vec![N::leaf("Version", vec![P::I32(100)])];
    layer_kids.extend(layer_refs);
    geometry_kids.push(N::tree("Layer", vec![P::I32(0)], layer_kids));

    let mut objects = vec![
        N::tree(
            "Geometry",
            vec![
                P::I64(ID_GEOMETRY),
                P::Str(object_name("terrain", "Geometry")),
                P::s("Mesh"),
            ],
            geometry_kids,
        ),
        N::tree(
            "Model",
            vec![
                P::I64(ID_MODEL),
                P::Str(object_name("terrain", "Model")),
                P::s("Mesh"),
            ],
            vec![
                N::leaf("Version", vec![P::I32(232)]),
                N::tree(
                    "Properties70",
                    vec![],
                    vec![
                        p70("RotationOrder", "enum", "", vec![P::I32(0)]),
                        p70("InheritType", "enum", "", vec![P::I32(1)]),
                        // Which attribute the Model shows; without it SDK-based importers read a Null.
                        p70_int("DefaultAttributeIndex", 0),
                        p70_vec("Lcl Translation", "Lcl Translation", [0.0; 3]),
                        p70_vec("Lcl Rotation", "Lcl Rotation", [0.0; 3]),
                        p70_vec("Lcl Scaling", "Lcl Scaling", [1.0; 3]),
                    ],
                ),
                N::leaf("Shading", vec![P::Bool(true)]),
                N::leaf("Culling", vec![P::s("CullingOff")]),
            ],
        ),
        N::tree(
            "Material",
            vec![
                P::I64(ID_MATERIAL),
                P::Str(object_name("terrain", "Material")),
                P::s(""),
            ],
            vec![
                N::leaf("Version", vec![P::I32(102)]),
                N::leaf("ShadingModel", vec![P::s("Phong")]),
                N::leaf("MultiLayer", vec![P::I32(0)]),
                N::tree(
                    "Properties70",
                    vec![],
                    vec![
                        p70_color(
                            "DiffuseColor",
                            if texture { [1.0; 3] } else { [0.55, 0.5, 0.45] },
                        ),
                        p70_color("SpecularColor", [0.05; 3]),
                        p70_f64("SpecularFactor", 0.05),
                        p70_f64("ShininessExponent", 2.0),
                        p70_f64("Opacity", 1.0),
                    ],
                ),
            ],
        ),
    ];
    let mut connections = vec![
        connection_oo(ID_MODEL, 0),
        connection_oo(ID_GEOMETRY, ID_MODEL),
        connection_oo(ID_MATERIAL, ID_MODEL),
    ];
    let mut definitions = vec![
        object_type("GlobalSettings", 1, None),
        object_type("Geometry", 1, Some("FbxMesh")),
        object_type("Model", 1, Some("FbxNode")),
        object_type("Material", 1, Some("FbxSurfacePhong")),
    ];
    if texture {
        let [min_h, max_h] = mesh.height_range;
        let sample = |gx: u32, gz: u32| mesh.positions[((gz * gw + gx) * 3 + 1) as usize];
        let normal_y = |gx: u32, gz: u32| mesh.normals[((gz * gw + gx) * 3 + 1) as usize];
        let png = bake_albedo(&sample, &normal_y, gw, gh, min_h, max_h)?;
        let file = "terrain-albedo.png";
        objects.push(N::tree(
            "Video",
            vec![
                P::I64(ID_VIDEO),
                P::Str(object_name(file, "Video")),
                P::s("Clip"),
            ],
            vec![
                N::leaf("Type", vec![P::s("Clip")]),
                N::tree(
                    "Properties70",
                    vec![],
                    vec![
                        p70("Path", "KString", "XRefUrl", vec![P::s(file)]),
                        p70("RelPath", "KString", "XRefUrl", vec![P::s(file)]),
                    ],
                ),
                N::leaf("UseMipMap", vec![P::I32(0)]),
                N::leaf("Filename", vec![P::s(file)]),
                N::leaf("RelativeFilename", vec![P::s(file)]),
                N::leaf("Content", vec![P::Raw(png)]),
            ],
        ));
        objects.push(N::tree(
            "Texture",
            vec![
                P::I64(ID_TEXTURE),
                P::Str(object_name(file, "Texture")),
                P::s(""),
            ],
            vec![
                N::leaf("Type", vec![P::s("TextureVideoClip")]),
                N::leaf("Version", vec![P::I32(202)]),
                N::leaf("TextureName", vec![P::Str(object_name(file, "Texture"))]),
                N::leaf("Media", vec![P::Str(object_name(file, "Video"))]),
                N::leaf("FileName", vec![P::s(file)]),
                N::leaf("RelativeFilename", vec![P::s(file)]),
                N::leaf("ModelUVTranslation", vec![P::F64(0.0), P::F64(0.0)]),
                N::leaf("ModelUVScaling", vec![P::F64(1.0), P::F64(1.0)]),
                N::leaf("Texture_Alpha_Source", vec![P::s("None")]),
                N::leaf("Cropping", vec![P::I32(0), P::I32(0), P::I32(0), P::I32(0)]),
            ],
        ));
        connections.push(connection_oo(ID_VIDEO, ID_TEXTURE));
        connections.push(connection_op(ID_TEXTURE, ID_MATERIAL, "DiffuseColor"));
        definitions.push(object_type("Texture", 1, Some("FbxFileTexture")));
        definitions.push(object_type("Video", 1, Some("FbxVideo")));
    }
    let object_count = definitions.len() as i32;
    definitions.insert(0, N::leaf("Count", vec![P::I32(object_count)]));
    definitions.insert(0, N::leaf("Version", vec![P::I32(100)]));

    let top = vec![
        N::tree(
            "FBXHeaderExtension",
            vec![],
            vec![
                N::leaf("FBXHeaderVersion", vec![P::I32(1003)]),
                N::leaf("FBXVersion", vec![P::I32(VERSION as i32)]),
                N::leaf("Creator", vec![P::s(&creator)]),
                N::tree(
                    "CreationTimeStamp",
                    vec![],
                    [
                        ("Version", 1000),
                        ("Year", 1970),
                        ("Month", 1),
                        ("Day", 1),
                        ("Hour", 10),
                        ("Minute", 0),
                        ("Second", 0),
                        ("Millisecond", 0),
                    ]
                    .into_iter()
                    .map(|(k, v)| N::leaf(k, vec![P::I32(v)]))
                    .collect(),
                ),
            ],
        ),
        N::leaf("FileId", vec![P::Raw(FILE_ID.to_vec())]),
        N::leaf("CreationTime", vec![P::s(CREATION_TIME)]),
        N::leaf("Creator", vec![P::s(&creator)]),
        N::tree(
            "GlobalSettings",
            vec![],
            vec![
                N::leaf("Version", vec![P::I32(1000)]),
                N::tree(
                    "Properties70",
                    vec![],
                    vec![
                        p70_int("UpAxis", 1),
                        p70_int("UpAxisSign", 1),
                        p70_int("FrontAxis", 2),
                        p70_int("FrontAxisSign", 1),
                        p70_int("CoordAxis", 0),
                        p70_int("CoordAxisSign", 1),
                        p70_f64("UnitScaleFactor", 100.0), // 1 unit = 100 cm: the mesh is in metres
                        p70_f64("OriginalUnitScaleFactor", 100.0),
                    ],
                ),
            ],
        ),
        N::tree(
            "Documents",
            vec![],
            vec![
                N::leaf("Count", vec![P::I32(1)]),
                N::tree(
                    "Document",
                    vec![P::I64(999_999_999), P::s("Scene"), P::s("Scene")],
                    vec![
                        N::tree(
                            "Properties70",
                            vec![],
                            vec![
                                p70("SourceObject", "object", "", vec![]),
                                p70("ActiveAnimStackName", "KString", "", vec![P::s("")]),
                            ],
                        ),
                        N::leaf("RootNode", vec![P::I64(0)]),
                    ],
                ),
            ],
        ),
        N::leaf("References", vec![]),
        N::tree("Definitions", vec![], definitions),
        N::tree("Objects", vec![], objects),
        N::tree("Connections", vec![], connections),
    ];

    let mut out = Vec::with_capacity(mesh.positions.len() * 8 * 3 + 4096);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    for node in &top {
        node.write(&mut out);
    }
    out.extend_from_slice(&[0u8; 13]);
    // Footer: id, zero padding to a 16-byte boundary (at least 4 bytes), version, 120 zeros, magic.
    out.extend_from_slice(&FOOTER_ID);
    out.extend_from_slice(&[0u8; 4]);
    while !out.len().is_multiple_of(16) {
        out.push(0);
    }
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&[0u8; 120]);
    out.extend_from_slice(&FOOTER_MAGIC);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, &out)?;
    Ok(MeshExportReport {
        path: path.to_path_buf(),
        format: "fbx".into(),
        vertices: n_vertices,
        triangles: n_quads * 2,
        step,
        bytes: out.len() as u64,
        extent_m,
        texture_px: texture.then_some([gw, gh]),
        normal_px: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Walk the node tree we wrote with an independent little reader: every record's end
    /// offset must land exactly where the next record starts, and the names must be the ones
    /// an importer looks for.
    fn walk(bytes: &[u8], mut pos: usize, end: usize, names: &mut Vec<String>) -> usize {
        while pos + 13 <= end {
            let u32_at =
                |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap()) as usize;
            let (node_end, n_props, props_len) = (u32_at(pos), u32_at(pos + 4), u32_at(pos + 8));
            let name_len = bytes[pos + 12] as usize;
            if node_end == 0 && n_props == 0 && props_len == 0 && name_len == 0 {
                return pos + 13; // null record
            }
            let name = std::str::from_utf8(&bytes[pos + 13..pos + 13 + name_len]).unwrap();
            names.push(name.to_owned());
            let body = pos + 13 + name_len + props_len;
            if body < node_end {
                let after = walk(bytes, body, node_end, names);
                assert_eq!(after, node_end, "children of {name} end at the record end");
            } else {
                assert_eq!(body, node_end, "leaf {name} ends at its record end");
            }
            pos = node_end;
        }
        pos
    }

    #[test]
    fn fbx_records_are_consistent_and_hold_a_quad_mesh() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut hf = Heightfield::new(9, 7, 8, 0).expect("hf");
        hf.fill_with(|x, y| (x as f32) * 2.0 + (y as f32) * 0.5);
        let p = dir.path().join("t.fbx");
        let r = export_fbx(&hf, [90.0, 70.0], &p, 1, true).expect("export");
        assert_eq!(
            (r.vertices, r.triangles, r.format.as_str()),
            (63, 96, "fbx")
        );
        let bytes = fs::read(&p).expect("read");
        assert_eq!(bytes.len() as u64, r.bytes);
        assert_eq!(&bytes[..23], MAGIC);
        assert_eq!(u32::from_le_bytes(bytes[23..27].try_into().unwrap()), 7400);
        let mut names = Vec::new();
        let after_top = walk(&bytes, 27, bytes.len(), &mut names);
        // Footer: id right after the top-level null record; version at a 16-byte boundary.
        assert_eq!(&bytes[after_top..after_top + 16], &FOOTER_ID);
        assert_eq!(&bytes[bytes.len() - 16..], &FOOTER_MAGIC);
        let version_at = bytes.len() - 16 - 120 - 4;
        assert_eq!(version_at % 16, 0);
        assert_eq!(
            u32::from_le_bytes(bytes[version_at..version_at + 4].try_into().unwrap()),
            7400
        );
        for needed in [
            "FBXHeaderExtension",
            "GlobalSettings",
            "Definitions",
            "Objects",
            "Geometry",
            "Vertices",
            "PolygonVertexIndex",
            "LayerElementNormal",
            "LayerElementUV",
            "Model",
            "Material",
            "Video",
            "Texture",
            "Content",
            "Connections",
        ] {
            assert!(names.iter().any(|n| n == needed), "missing node {needed}");
        }
        assert_eq!(
            names.iter().filter(|n| *n == "C").count(),
            5,
            "model, geometry, material, video, texture links"
        );
        // Vertices: 63 × 3 doubles; polygons: 48 quads, every 4th index negative.
        let vi = bytes.windows(8).position(|w| w == b"Vertices").unwrap();
        let count = u32::from_le_bytes(bytes[vi + 9..vi + 13].try_into().unwrap());
        assert_eq!(count, 63 * 3);
        let pi = bytes
            .windows(18)
            .position(|w| w == b"PolygonVertexIndex")
            .unwrap();
        let n = u32::from_le_bytes(bytes[pi + 19..pi + 23].try_into().unwrap()) as usize;
        assert_eq!(n, 48 * 4);
        let data = pi + 19 + 12;
        let idx: Vec<i32> = (0..n)
            .map(|k| i32::from_le_bytes(bytes[data + 4 * k..data + 4 * k + 4].try_into().unwrap()))
            .collect();
        assert!(
            idx.chunks(4)
                .all(|q| q[0] >= 0 && q[1] >= 0 && q[2] >= 0 && q[3] < 0)
        );
        assert_eq!(&idx[..4], &[0, 1, 10, -10]); // a=0, b=1, d=10, c=9 → -(9+1)
        assert!(
            export_fbx(&hf, [90.0, 70.0], &dir.path().join("flat.fbx"), 1, false)
                .expect("no texture")
                .bytes
                < r.bytes
        );
    }
}
