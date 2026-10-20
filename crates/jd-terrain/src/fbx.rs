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
        lods: Vec::new(),
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

// ───────────────────────────── Reader ─────────────────────────────
//
// Our own parser for Kaydara FBX Binary 7.x (32-bit headers before 7500, 64-bit from 7500),
// raw or zlib-compressed arrays. It reads what a terrain tool needs back from a DCC: every
// `Geometry`/`Mesh` with its `Model` transform, `GlobalSettings` units and up axis, then
// rasterises the mesh onto a height grid. No materials, animation or skinning.

/// A parsed property (only the kinds the reader uses are inspected further).
enum RP {
    I32(i32),
    I64(i64),
    F64(f64),
    Str(String),
    F64s(Vec<f64>),
    I32s(Vec<i32>),
    Other,
}

struct RN {
    name: String,
    props: Vec<RP>,
    kids: Vec<RN>,
}

fn corrupt(what: &str) -> jd_types::JdError {
    jd_types::JdError::new(
        jd_types::ErrorCode::E_FORMAT_CORRUPT,
        format!("FBX: {what}"),
    )
}

fn le_u32(b: &[u8], at: usize) -> JdResult<u32> {
    b.get(at..at + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| corrupt("truncated record"))
}
fn le_u64(b: &[u8], at: usize) -> JdResult<u64> {
    b.get(at..at + 8)
        .map(|s| u64::from_le_bytes(s.try_into().unwrap_or([0; 8])))
        .ok_or_else(|| corrupt("truncated record"))
}

/// Array property body: count, encoding, byte length, data (zlib when encoding == 1).
fn read_array(b: &[u8], pos: usize, elem: usize) -> JdResult<(Vec<u8>, usize, usize)> {
    let (count, encoding, len) = (
        le_u32(b, pos)? as usize,
        le_u32(b, pos + 4)?,
        le_u32(b, pos + 8)? as usize,
    );
    let data = b
        .get(pos + 12..pos + 12 + len)
        .ok_or_else(|| corrupt("truncated array"))?;
    let raw = match encoding {
        0 => {
            if data.len() != count.saturating_mul(elem) {
                return Err(corrupt("array length does not match its count"));
            }
            data.to_vec()
        }
        1 => {
            use std::io::Read as _;
            // Bounded: a crafted header could claim gigabytes; the decoded stream is capped at
            // the declared size + 1 so an over-long stream is detected, not swallowed.
            let want = count
                .checked_mul(elem)
                .ok_or_else(|| corrupt("array size overflows"))?;
            if want > MAX_ARRAY_BYTES {
                return Err(corrupt("array larger than 1 GiB"));
            }
            let mut out = Vec::with_capacity(want.min(1 << 20));
            flate2::read::ZlibDecoder::new(data)
                .take(want as u64 + 1)
                .read_to_end(&mut out)
                .map_err(|e| corrupt(&format!("zlib array: {e}")))?;
            out
        }
        other => return Err(corrupt(&format!("unknown array encoding {other}"))),
    };
    if raw.len() != count * elem {
        return Err(corrupt("array length does not match its count"));
    }
    Ok((raw, count, pos + 12 + len))
}

fn parse_prop(b: &[u8], pos: usize) -> JdResult<(RP, usize)> {
    let tag = *b.get(pos).ok_or_else(|| corrupt("truncated property"))?;
    let p = pos + 1;
    Ok(match tag {
        b'Y' => (RP::Other, p + 2),
        b'C' => (RP::Other, p + 1),
        b'I' => (RP::I32(le_u32(b, p)? as i32), p + 4),
        b'F' => (RP::Other, p + 4),
        b'D' => (RP::F64(f64::from_bits(le_u64(b, p)?)), p + 8),
        b'L' => (RP::I64(le_u64(b, p)? as i64), p + 8),
        b'S' | b'R' => {
            let len = le_u32(b, p)? as usize;
            let bytes = b
                .get(p + 4..p + 4 + len)
                .ok_or_else(|| corrupt("truncated string"))?;
            let v = if tag == b'S' {
                RP::Str(String::from_utf8_lossy(bytes).into_owned())
            } else {
                RP::Other
            };
            (v, p + 4 + len)
        }
        b'd' => {
            let (raw, n, end) = read_array(b, p, 8)?;
            (
                RP::F64s(
                    (0..n)
                        .map(|i| {
                            f64::from_le_bytes(raw[i * 8..i * 8 + 8].try_into().unwrap_or([0; 8]))
                        })
                        .collect(),
                ),
                end,
            )
        }
        b'f' => {
            let (raw, n, end) = read_array(b, p, 4)?;
            (
                RP::F64s(
                    (0..n)
                        .map(|i| {
                            f64::from(f32::from_le_bytes(
                                raw[i * 4..i * 4 + 4].try_into().unwrap_or([0; 4]),
                            ))
                        })
                        .collect(),
                ),
                end,
            )
        }
        b'i' => {
            let (raw, n, end) = read_array(b, p, 4)?;
            (
                RP::I32s(
                    (0..n)
                        .map(|i| {
                            i32::from_le_bytes(raw[i * 4..i * 4 + 4].try_into().unwrap_or([0; 4]))
                        })
                        .collect(),
                ),
                end,
            )
        }
        b'l' => {
            let (_, _, end) = read_array(b, p, 8)?;
            (RP::Other, end)
        }
        b'b' => {
            let (_, _, end) = read_array(b, p, 1)?;
            (RP::Other, end)
        }
        other => {
            return Err(corrupt(&format!(
                "unknown property type {:?}",
                other as char
            )));
        }
    })
}

/// One record at `pos`; `None` for a null record. `big` = 64-bit header fields (≥ 7500).
/// Largest array a record may declare (1 GiB).
const MAX_ARRAY_BYTES: usize = 1 << 30;

/// Deepest record nesting accepted (real files stay under ~10; a crafted one could otherwise
/// recurse until the stack overflows).
const MAX_DEPTH: u32 = 64;

fn parse_node(b: &[u8], pos: usize, big: bool, depth: u32) -> JdResult<Option<(RN, usize)>> {
    if depth > MAX_DEPTH {
        return Err(corrupt("records nested deeper than 64 levels"));
    }
    let (end, n_props, props_len, mut p) = if big {
        (
            le_u64(b, pos)? as usize,
            le_u64(b, pos + 8)? as usize,
            le_u64(b, pos + 16)? as usize,
            pos + 24,
        )
    } else {
        (
            le_u32(b, pos)? as usize,
            le_u32(b, pos + 4)? as usize,
            le_u32(b, pos + 8)? as usize,
            pos + 12,
        )
    };
    let name_len = *b.get(p).ok_or_else(|| corrupt("truncated record"))? as usize;
    if end == 0 && n_props == 0 && props_len == 0 && name_len == 0 {
        return Ok(None);
    }
    p += 1;
    let name = String::from_utf8_lossy(
        b.get(p..p + name_len)
            .ok_or_else(|| corrupt("truncated name"))?,
    )
    .into_owned();
    p += name_len;
    let mut props = Vec::with_capacity(n_props.min(64));
    for _ in 0..n_props {
        let (v, next) = parse_prop(b, p)?;
        props.push(v);
        p = next;
    }
    let mut kids = Vec::new();
    while p < end {
        match parse_node(b, p, big, depth + 1)? {
            Some((kid, next)) => {
                kids.push(kid);
                p = next;
            }
            None => {
                p = end;
            }
        }
    }
    if p != end {
        return Err(corrupt(&format!("record `{name}` overruns its end offset")));
    }
    Ok(Some((RN { name, props, kids }, end)))
}

fn parse_document(b: &[u8]) -> JdResult<Vec<RN>> {
    if b.len() < 27 || b[..21] != MAGIC[..21] {
        return Err(jd_types::JdError::new(
            jd_types::ErrorCode::E_FORMAT_UNSUPPORTED,
            "not a binary FBX file",
        )
        .next("ASCII FBX is not supported; export as binary (every DCC offers it) or as glTF."));
    }
    let version = le_u32(b, 23)?;
    let big = version >= 7500;
    let mut pos = 27;
    let mut top = Vec::new();
    while pos < b.len() {
        match parse_node(b, pos, big, 0)? {
            Some((node, next)) => {
                top.push(node);
                pos = next;
            }
            None => break,
        }
    }
    Ok(top)
}

impl RN {
    fn child(&self, name: &str) -> Option<&RN> {
        self.kids.iter().find(|k| k.name == name)
    }
    fn f64s(&self) -> Option<&[f64]> {
        self.props.iter().find_map(|p| {
            if let RP::F64s(v) = p {
                Some(v.as_slice())
            } else {
                None
            }
        })
    }
    fn i32s(&self) -> Option<&[i32]> {
        self.props.iter().find_map(|p| {
            if let RP::I32s(v) = p {
                Some(v.as_slice())
            } else {
                None
            }
        })
    }
    fn str_at(&self, i: usize) -> Option<&str> {
        match self.props.get(i) {
            Some(RP::Str(s)) => Some(s),
            _ => None,
        }
    }
    fn id(&self) -> Option<i64> {
        match self.props.first() {
            Some(RP::I64(v)) => Some(*v),
            Some(RP::I32(v)) => Some(i64::from(*v)),
            _ => None,
        }
    }
    /// `Properties70` entry `name` → its numeric values (after the four header strings).
    fn p70(&self, name: &str) -> Option<Vec<f64>> {
        let props = self.child("Properties70")?;
        let p = props
            .kids
            .iter()
            .find(|k| k.name == "P" && k.str_at(0) == Some(name))?;
        let nums: Vec<f64> = p
            .props
            .iter()
            .skip(4)
            .filter_map(|v| match v {
                RP::F64(x) => Some(*x),
                RP::I32(x) => Some(f64::from(*x)),
                RP::I64(x) => Some(*x as f64),
                _ => None,
            })
            .collect();
        Some(nums)
    }
}

/// A mesh read from an FBX file: positions in metres in the terrain frame (+X east, +Y up,
/// +Z south), triangles as vertex indices.
#[derive(Debug)]
pub struct FbxMesh {
    /// Vertex positions.
    pub positions: Vec<[f64; 3]>,
    /// Triangle indices.
    pub triangles: Vec<[u32; 3]>,
}

/// Rotation (degrees, XYZ order), scale and translation applied to a point.
fn transform(p: [f64; 3], t: &[f64], r: &[f64], s: &[f64]) -> [f64; 3] {
    let [mut x, mut y, mut z] = p;
    if s.len() == 3 {
        x *= s[0];
        y *= s[1];
        z *= s[2];
    }
    if r.len() == 3 {
        let (sx, cx) = r[0].to_radians().sin_cos();
        let (sy, cy) = r[1].to_radians().sin_cos();
        let (sz, cz) = r[2].to_radians().sin_cos();
        // X, then Y, then Z (FBX default rotation order).
        let (y1, z1) = (y * cx - z * sx, y * sx + z * cx);
        let (x2, z2) = (x * cy + z1 * sy, -x * sy + z1 * cy);
        let (x3, y3) = (x2 * cz - y1 * sz, x2 * sz + y1 * cz);
        x = x3;
        y = y3;
        z = z2;
    }
    if t.len() == 3 {
        x += t[0];
        y += t[1];
        z += t[2];
    }
    [x, y, z]
}

/// Read every mesh of a binary FBX file into one triangle soup in metres, +Y up.
pub fn read_fbx_mesh(path: &Path) -> JdResult<FbxMesh> {
    read_fbx_mesh_bytes(&fs::read(path)?)
}

/// Parse a binary FBX document from memory (the fuzz target's entry point; `read_fbx_mesh` is
/// this over a file).
pub fn read_fbx_mesh_bytes(bytes: &[u8]) -> JdResult<FbxMesh> {
    let top = parse_document(bytes)?;
    let find = |name: &str| top.iter().find(|n| n.name == name);
    let settings = find("GlobalSettings");
    let unit = settings
        .and_then(|g| g.p70("UnitScaleFactor"))
        .and_then(|v| v.first().copied())
        .unwrap_or(1.0);
    let up_axis = settings
        .and_then(|g| g.p70("UpAxis"))
        .and_then(|v| v.first().copied())
        .unwrap_or(1.0) as i32;
    let up_sign = settings
        .and_then(|g| g.p70("UpAxisSign"))
        .and_then(|v| v.first().copied())
        .unwrap_or(1.0);
    let to_m = unit / 100.0;
    let objects = find("Objects").ok_or_else(|| corrupt("no Objects section"))?;
    // Geometry → Model links (`C "OO" child parent`), so each mesh gets its node's transform.
    let mut parent_of = std::collections::HashMap::new();
    if let Some(conn) = find("Connections") {
        for c in conn
            .kids
            .iter()
            .filter(|c| c.name == "C" && c.str_at(0) == Some("OO"))
        {
            if let (Some(RP::I64(child)), Some(RP::I64(parent))) = (c.props.get(1), c.props.get(2))
            {
                parent_of.insert(*child, *parent);
            }
        }
    }
    let models: std::collections::HashMap<i64, &RN> = objects
        .kids
        .iter()
        .filter(|n| n.name == "Model")
        .filter_map(|m| m.id().map(|id| (id, m)))
        .collect();
    let mut mesh = FbxMesh {
        positions: Vec::new(),
        triangles: Vec::new(),
    };
    for geom in objects
        .kids
        .iter()
        .filter(|n| n.name == "Geometry" && n.str_at(2) == Some("Mesh"))
    {
        let verts = geom
            .child("Vertices")
            .and_then(RN::f64s)
            .ok_or_else(|| corrupt("Geometry without Vertices"))?;
        let poly = geom
            .child("PolygonVertexIndex")
            .and_then(RN::i32s)
            .ok_or_else(|| corrupt("Geometry without PolygonVertexIndex"))?;
        let model = geom
            .id()
            .and_then(|id| parent_of.get(&id))
            .and_then(|m| models.get(m));
        let (t, r, s) = match model {
            Some(m) => (
                m.p70("Lcl Translation").unwrap_or_default(),
                m.p70("Lcl Rotation").unwrap_or_default(),
                m.p70("Lcl Scaling").unwrap_or_default(),
            ),
            None => (Vec::new(), Vec::new(), Vec::new()),
        };
        let base = mesh.positions.len() as u32;
        for v in verts.as_chunks::<3>().0 {
            let [x, y, z] = transform([v[0], v[1], v[2]], &t, &r, &s);
            // Z-up files (3ds Max, unconverted Blender): swing the up axis onto +Y.
            let p = if up_axis == 2 {
                [x, z * up_sign, -y]
            } else {
                [x, y * up_sign, z]
            };
            mesh.positions.push([p[0] * to_m, p[1] * to_m, p[2] * to_m]);
        }
        let n = verts.len() / 3;
        let mut face: Vec<u32> = Vec::with_capacity(4);
        for &i in poly {
            let (idx, last) = if i < 0 {
                ((-i - 1) as usize, true)
            } else {
                (i as usize, false)
            };
            if idx >= n {
                return Err(corrupt("polygon index out of range"));
            }
            face.push(base + idx as u32);
            if last {
                for k in 1..face.len().saturating_sub(1) {
                    mesh.triangles.push([face[0], face[k], face[k + 1]]);
                }
                face.clear();
            }
        }
    }
    if mesh.triangles.is_empty() {
        return Err(corrupt("no mesh geometry found"));
    }
    Ok(mesh)
}

/// Rasterise an FBX mesh onto a height grid over its own footprint: `size_px` cells (default
/// ≈ one cell per vertex, square cells), row 0 = north, cells no triangle covers filled with
/// the lowest height. Returns `(heights, width, height)` like the other foreign readers.
pub fn read_fbx_heightfield(
    path: &Path,
    size_px: Option<[u32; 2]>,
) -> JdResult<(Vec<f32>, u32, u32)> {
    let mesh = read_fbx_mesh(path)?;
    let (mut min, mut max) = ([f64::MAX; 3], [f64::MIN; 3]);
    for p in &mesh.positions {
        for k in 0..3 {
            min[k] = min[k].min(p[k]);
            max[k] = max[k].max(p[k]);
        }
    }
    let (ex, ez) = ((max[0] - min[0]).max(1e-9), (max[2] - min[2]).max(1e-9));
    let [w, h] = size_px.unwrap_or_else(|| {
        // About one cell per vertex, split by the footprint's aspect ratio.
        let n = mesh.positions.len() as f64;
        let w = (n * ex / ez).sqrt().round().clamp(2.0, 16384.0);
        [w as u32, (n / w).round().clamp(2.0, 16384.0) as u32]
    });
    if w < 2 || h < 2 {
        return Err(jd_types::JdError::invalid(
            "size_px",
            "needs at least 2×2 cells",
        ));
    }
    // Cell centres span the footprint edge to edge: centre (i, j) = min + i · extent / (n − 1),
    // so a regular mesh grid read at its own size puts every centre on a vertex (exact heights).
    // Row 0 = north = the smallest Z (north is −Z).
    let (cw, ch) = (ex / (w - 1) as f64, ez / (h - 1) as f64);
    let mut grid = vec![f32::NAN; (w * h) as usize];
    for tri in &mesh.triangles {
        let [a, b, c] = tri.map(|i| mesh.positions[i as usize]);
        let xs = [a[0], b[0], c[0]];
        let zs = [a[2], b[2], c[2]];
        let (x0, x1) = (
            xs.iter().cloned().fold(f64::MAX, f64::min),
            xs.iter().cloned().fold(f64::MIN, f64::max),
        );
        let (z0, z1) = (
            zs.iter().cloned().fold(f64::MAX, f64::min),
            zs.iter().cloned().fold(f64::MIN, f64::max),
        );
        let (i0, i1) = (
            (((x0 - min[0]) / cw).floor().max(0.0)) as i64,
            (((x1 - min[0]) / cw).ceil().min(w as f64 - 1.0)) as i64,
        );
        let (j0, j1) = (
            (((z0 - min[2]) / ch).floor().max(0.0)) as i64,
            (((z1 - min[2]) / ch).ceil().min(h as f64 - 1.0)) as i64,
        );
        let det = (b[0] - a[0]) * (c[2] - a[2]) - (c[0] - a[0]) * (b[2] - a[2]);
        if det.abs() < 1e-18 {
            continue;
        }
        for j in j0..=j1 {
            let pz = min[2] + j as f64 * ch;
            for i in i0..=i1 {
                let px = min[0] + i as f64 * cw;
                let l1 = ((b[0] - px) * (c[2] - pz) - (c[0] - px) * (b[2] - pz)) / det;
                let l2 = ((c[0] - px) * (a[2] - pz) - (a[0] - px) * (c[2] - pz)) / det;
                let l3 = 1.0 - l1 - l2;
                // Slack of ~1e-3 of a triangle: DCCs store positions as f32 centimetres, so
                // boundary vertices jitter by ~0.2 mm and a strict test left edge cells uncovered.
                let eps = -1e-3;
                if l1 >= eps && l2 >= eps && l3 >= eps {
                    grid[(j as u32 * w + i as u32) as usize] =
                        (l1 * a[1] + l2 * b[1] + l3 * c[1]) as f32;
                }
            }
        }
    }
    let floor = grid
        .iter()
        .cloned()
        .filter(|v| v.is_finite())
        .fold(f32::MAX, f32::min);
    if floor == f32::MAX {
        return Err(corrupt("mesh covers no grid cell"));
    }
    for v in &mut grid {
        if !v.is_finite() {
            *v = floor;
        }
    }
    Ok((grid, w, h))
}

#[cfg(test)]
mod read_tests {
    use super::*;
    use proptest::prelude::*;

    fn small_fbx() -> Vec<u8> {
        let dir = tempfile::tempdir().expect("tmp");
        let mut hf = Heightfield::new(6, 5, 8, 0).expect("hf");
        hf.fill_with(|x, y| 10.0 + x as f32 + y as f32 * 0.5);
        let p = dir.path().join("f.fbx");
        export_fbx(&hf, [60.0, 50.0], &p, 1, false).expect("export");
        fs::read(&p).expect("read")
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]
        /// Any corruption of a valid file (flipped bytes, truncation, a spliced-in huge array
        /// header) ends in Ok or a clean Err — never a panic, never a runaway allocation.
        #[test]
        fn mutated_fbx_never_panics(
            edits in prop::collection::vec((0usize..4096, 0u8..=255u8), 1..12),
            cut in 0usize..4096,
            splice in prop::option::of(0usize..4096),
        ) {
            let mut bytes = small_fbx();
            for (at, v) in edits {
                let i = at % bytes.len();
                bytes[i] = v;
            }
            if let Some(at) = splice {
                // A float array claiming 2^31 elements, raw encoding.
                let i = at % bytes.len();
                let hdr = [b'd', 0, 0, 0, 0x80, 0, 0, 0, 0, 0, 0, 0, 0];
                let end = (i + hdr.len()).min(bytes.len());
                bytes[i..end].copy_from_slice(&hdr[..end - i]);
            }
            bytes.truncate((cut % bytes.len()).max(1));
            let dir = tempfile::tempdir().expect("tmp");
            let p = dir.path().join("m.fbx");
            fs::write(&p, &bytes).expect("write");
            let _ = read_fbx_mesh(&p).map(|m| m.triangles.len());
            let _ = read_fbx_heightfield(&p, Some([4, 4]));
        }
    }

    #[test]
    fn deep_nesting_and_huge_arrays_are_refused() {
        // 70 nested empty records (32-bit headers): each claims to end at the file end.
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&7400u32.to_le_bytes());
        let total = 27 + 70 * 14 + 13;
        for _ in 0..70 {
            bytes.extend_from_slice(&(total as u32).to_le_bytes());
            bytes.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 1, b'a']);
        }
        bytes.extend_from_slice(&[0u8; 13]);
        let dir = tempfile::tempdir().expect("tmp");
        let p = dir.path().join("deep.fbx");
        fs::write(&p, &bytes).expect("write");
        let err = read_fbx_mesh(&p).expect_err("too deep");
        assert!(err.message.contains("nested"), "{}", err.message);
        // A record with a 1-element f64 array that declares 1.5 GiB.
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&7400u32.to_le_bytes());
        let mut prop = vec![b'd'];
        prop.extend_from_slice(&((1usize << 28) as u32).to_le_bytes()); // 2^28 doubles = 2 GiB
        prop.extend_from_slice(&1u32.to_le_bytes()); // zlib
        prop.extend_from_slice(&4u32.to_le_bytes());
        prop.extend_from_slice(&[0x78, 0x9c, 0, 0]);
        let rec_len = 13 + 1 + prop.len();
        bytes.extend_from_slice(&((27 + rec_len) as u32).to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&(prop.len() as u32).to_le_bytes());
        bytes.push(1);
        bytes.push(b'V');
        bytes.extend_from_slice(&prop);
        bytes.extend_from_slice(&[0u8; 13]);
        fs::write(&p, &bytes).expect("write");
        let err = read_fbx_mesh(&p).expect_err("too big");
        assert!(
            err.message.contains("larger")
                || err.message.contains("length")
                || err.message.contains("zlib"),
            "{}",
            err.message
        );
    }

    #[test]
    fn our_fbx_reads_back_to_the_same_heights() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut hf = Heightfield::new(16, 12, 8, 0).expect("hf");
        hf.fill_with(|x, y| ((x as f32) * 0.7).sin() * 30.0 + (y as f32) * 2.0 + 100.0);
        let p = dir.path().join("rt.fbx");
        export_fbx(&hf, [160.0, 120.0], &p, 1, false).expect("export");
        let mesh = read_fbx_mesh(&p).expect("read");
        assert_eq!(
            (mesh.positions.len(), mesh.triangles.len()),
            (16 * 12, 15 * 11 * 2)
        );
        // Grid of the same size: every cell centre is a vertex, so heights come back exactly.
        let (data, w, h) = read_fbx_heightfield(&p, Some([16, 12])).expect("raster");
        assert_eq!((w, h), (16, 12));
        let mut worst = 0.0f32;
        for y in 0..12 {
            for x in 0..16 {
                // Row 0 of the result is north = the field's last row.
                let want = hf.get(x, 11 - y);
                worst = worst.max((data[(y * 16 + x) as usize] - want).abs());
            }
        }
        assert!(worst < 1e-3, "max height error {worst}");
        // Default sizing lands near one cell per vertex; a coarser grid still samples the surface.
        let (_, w2, h2) = read_fbx_heightfield(&p, None).expect("auto");
        assert!(
            (14..=18).contains(&w2) && (10..=14).contains(&h2),
            "{w2}×{h2}"
        );
        let (coarse, _, _) = read_fbx_heightfield(&p, Some([8, 6])).expect("coarse");
        assert!(coarse.iter().all(|v| (60.0..=160.0).contains(v)));
        assert!(read_fbx_mesh(&dir.path().join("missing.fbx")).is_err());
        fs::write(dir.path().join("ascii.fbx"), b"; FBX 7.4.0 project file\n").expect("write");
        assert_eq!(
            read_fbx_mesh(&dir.path().join("ascii.fbx"))
                .err()
                .map(|e| e.code),
            Some(jd_types::ErrorCode::E_FORMAT_UNSUPPORTED)
        );
    }
}
