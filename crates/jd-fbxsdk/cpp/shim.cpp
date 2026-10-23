// jd-fbxsdk: a C shim over the Autodesk FBX SDK (read a mesh, write a mesh) for the Jenga Dunia
// test oracle. Exceptions never cross the boundary; every entry point returns 0 on success and a
// caller-freed error string otherwise. Built by the crate's build.rs when the SDK is installed.
#include <fbxsdk.h>

#include <cstdint>
#include <cstring>
#include <string>
#include <vector>

extern "C" {

struct jd_fbx_mesh {
    double* positions;      // 3 per control point
    uint32_t n_positions;   // control points
    uint32_t* triangles;    // 3 per triangle (after the SDK's triangulation)
    uint32_t n_triangles;
    uint32_t n_polygons;    // before triangulation
    uint32_t n_meshes;      // meshes in the scene (the first is reported)
    int32_t has_normals;
    int32_t has_uvs;
    int32_t file_version;   // e.g. 7400
    double unit_scale;      // FbxSystemUnit scale factor (cm = 1, m = 100)
    int32_t up_axis;        // 1 = Y, 2 = Z
    char* material;         // first material name or NULL
    char* texture;          // first file texture's relative name or NULL
};

static char* dup(const std::string& s) {
    char* out = static_cast<char*>(std::malloc(s.size() + 1));
    if (out) std::memcpy(out, s.c_str(), s.size() + 1);
    return out;
}

const char* jd_fbxsdk_version(void) { return FbxManager::GetVersion(true); }

void jd_fbxsdk_free_str(char* s) { std::free(s); }

void jd_fbxsdk_free(jd_fbx_mesh* m) {
    if (!m) return;
    std::free(m->positions);
    std::free(m->triangles);
    std::free(m->material);
    std::free(m->texture);
    std::free(m);
}

int jd_fbxsdk_read(const char* path, jd_fbx_mesh** out, char** err) {
    *out = nullptr;
    *err = nullptr;
    FbxManager* manager = nullptr;
    try {
        manager = FbxManager::Create();
        if (!manager) { *err = dup("FbxManager::Create failed"); return 1; }
        FbxIOSettings* ios = FbxIOSettings::Create(manager, IOSROOT);
        ios->SetBoolProp(IMP_FBX_EXTRACT_EMBEDDED_DATA, false); // never write a .fbm folder next to the file
        manager->SetIOSettings(ios);
        FbxImporter* importer = FbxImporter::Create(manager, "");
        if (!importer->Initialize(path, -1, manager->GetIOSettings())) {
            *err = dup(std::string("Initialize: ") + importer->GetStatus().GetErrorString());
            manager->Destroy();
            return 2;
        }
        int major = 0, minor = 0, rev = 0;
        importer->GetFileVersion(major, minor, rev);
        FbxScene* scene = FbxScene::Create(manager, "jd");
        if (!importer->Import(scene)) {
            *err = dup(std::string("Import: ") + importer->GetStatus().GetErrorString());
            manager->Destroy();
            return 3;
        }
        importer->Destroy();
        jd_fbx_mesh* m = static_cast<jd_fbx_mesh*>(std::calloc(1, sizeof(jd_fbx_mesh)));
        m->file_version = major * 1000 + minor * 100 + rev;
        m->unit_scale = scene->GetGlobalSettings().GetSystemUnit().GetScaleFactor();
        int sign = 1;
        m->up_axis = scene->GetGlobalSettings().GetAxisSystem().GetUpVector(sign) == FbxAxisSystem::eZAxis ? 2 : 1;
        m->n_meshes = static_cast<uint32_t>(scene->GetSrcObjectCount<FbxMesh>());
        if (m->n_meshes > 0) {
            FbxMesh* mesh = scene->GetSrcObject<FbxMesh>(0);
            m->n_polygons = static_cast<uint32_t>(mesh->GetPolygonCount());
            m->has_normals = mesh->GetElementNormalCount() > 0;
            m->has_uvs = mesh->GetElementUVCount() > 0;
            // Material and texture first: Triangulate(…, true) replaces the mesh object.
            FbxNode* node = mesh->GetNode();
            if (node && node->GetMaterialCount() > 0) {
                FbxSurfaceMaterial* mat = node->GetMaterial(0);
                if (mat) {
                    m->material = dup(mat->GetName());
                    FbxProperty prop = mat->FindProperty(FbxSurfaceMaterial::sDiffuse);
                    if (prop.IsValid() && prop.GetSrcObjectCount<FbxFileTexture>() > 0) {
                        FbxFileTexture* tex = prop.GetSrcObject<FbxFileTexture>(0);
                        if (tex) m->texture = dup(tex->GetRelativeFileName());
                    }
                }
            }
            // The SDK's own triangulation: what an engine's importer would see.
            FbxGeometryConverter conv(manager);
            FbxNodeAttribute* tri = conv.Triangulate(mesh, true);
            FbxMesh* tm = tri && tri->GetAttributeType() == FbxNodeAttribute::eMesh ? static_cast<FbxMesh*>(tri) : mesh;
            const int n = tm->GetControlPointsCount();
            m->n_positions = static_cast<uint32_t>(n);
            m->positions = static_cast<double*>(std::malloc(sizeof(double) * 3 * (n > 0 ? n : 1)));
            const FbxVector4* cp = tm->GetControlPoints();
            for (int i = 0; i < n; i++) {
                m->positions[i * 3] = cp[i][0];
                m->positions[i * 3 + 1] = cp[i][1];
                m->positions[i * 3 + 2] = cp[i][2];
            }
            std::vector<uint32_t> tris;
            const int pc = tm->GetPolygonCount();
            for (int p = 0; p < pc; p++) {
                const int sz = tm->GetPolygonSize(p);
                for (int k = 1; k + 1 < sz; k++) { // fan: the SDK already made triangles
                    tris.push_back(static_cast<uint32_t>(tm->GetPolygonVertex(p, 0)));
                    tris.push_back(static_cast<uint32_t>(tm->GetPolygonVertex(p, k)));
                    tris.push_back(static_cast<uint32_t>(tm->GetPolygonVertex(p, k + 1)));
                }
            }
            m->n_triangles = static_cast<uint32_t>(tris.size() / 3);
            m->triangles = static_cast<uint32_t*>(std::malloc(sizeof(uint32_t) * (tris.empty() ? 1 : tris.size())));
            if (!tris.empty()) std::memcpy(m->triangles, tris.data(), sizeof(uint32_t) * tris.size());
        }
        manager->Destroy();
        *out = m;
        return 0;
    } catch (const std::exception& e) {
        *err = dup(std::string("exception: ") + e.what());
    } catch (...) {
        *err = dup("unknown exception");
    }
    if (manager) manager->Destroy();
    return 9;
}

// Write a triangle mesh (positions in metres, Y up) as a binary FBX of `version` (7400 or 7500),
// with the SDK's default compressed arrays, in centimetres like every DCC expects.
int jd_fbxsdk_write(const char* path, const double* positions, uint32_t n_positions, const uint32_t* triangles,
                    uint32_t n_triangles, int version, char** err) {
    *err = nullptr;
    FbxManager* manager = nullptr;
    try {
        manager = FbxManager::Create();
        FbxIOSettings* ios = FbxIOSettings::Create(manager, IOSROOT);
        manager->SetIOSettings(ios);
        FbxScene* scene = FbxScene::Create(manager, "jd");
        scene->GetGlobalSettings().SetSystemUnit(FbxSystemUnit::cm);
        scene->GetGlobalSettings().SetAxisSystem(FbxAxisSystem::MayaYUp);
        FbxMesh* mesh = FbxMesh::Create(scene, "terrain");
        mesh->InitControlPoints(static_cast<int>(n_positions));
        FbxVector4* cp = mesh->GetControlPoints();
        for (uint32_t i = 0; i < n_positions; i++) {
            cp[i] = FbxVector4(positions[i * 3] * 100.0, positions[i * 3 + 1] * 100.0, positions[i * 3 + 2] * 100.0);
        }
        for (uint32_t t = 0; t < n_triangles; t++) {
            mesh->BeginPolygon();
            mesh->AddPolygon(static_cast<int>(triangles[t * 3]));
            mesh->AddPolygon(static_cast<int>(triangles[t * 3 + 1]));
            mesh->AddPolygon(static_cast<int>(triangles[t * 3 + 2]));
            mesh->EndPolygon();
        }
        mesh->GenerateNormals();
        FbxNode* node = FbxNode::Create(scene, "terrain");
        node->SetNodeAttribute(mesh);
        scene->GetRootNode()->AddChild(node);
        FbxExporter* exporter = FbxExporter::Create(manager, "");
        const int format = manager->GetIOPluginRegistry()->FindWriterIDByDescription("FBX binary (*.fbx)");
        if (!exporter->Initialize(path, format, manager->GetIOSettings())) {
            *err = dup(std::string("Initialize: ") + exporter->GetStatus().GetErrorString());
            manager->Destroy();
            return 2;
        }
        exporter->SetFileExportVersion(version >= 7500 ? FBX_2016_00_COMPATIBLE : FBX_2014_00_COMPATIBLE);
        if (!exporter->Export(scene)) {
            *err = dup(std::string("Export: ") + exporter->GetStatus().GetErrorString());
            manager->Destroy();
            return 3;
        }
        exporter->Destroy();
        manager->Destroy();
        return 0;
    } catch (const std::exception& e) {
        *err = dup(std::string("exception: ") + e.what());
    } catch (...) {
        *err = dup("unknown exception");
    }
    if (manager) manager->Destroy();
    return 9;
}

}  // extern "C"
