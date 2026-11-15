//! [`MeshAssets`]: a project's mesh and texture assets, loaded for the parts that name them (#189).
//!
//! `MeshPart`s name their mesh and texture by asset id (`MeshId`, `TextureId`). The scene numbers every pair it meets
//! as a mesh kind ([`InstanceScene::mesh_assets`]); [`MeshAssets::update`] asks a worker thread for each asset not
//! asked for yet, which reads it from the `.mtproj` ([`AssetReader`]) and decodes it (glTF or GLB, PNG or JPEG) off the
//! main thread, and hands what came back to the renderer ([`Renderer::add_mesh_asset`],
//! [`Renderer::add_texture_asset`]). The parts draw once their mesh is here, textured once their texture is.
//!
//! Materials load the same way (#678, ADR 0012): every material key parts name in `Material` (an asset id, or a
//! material's name: the game's material of that name, else the built-in one) is read once, however many parts use it,
//! and its parameters are given to the renderer's material table ([`metallic_render::MaterialTable::set_material`]).
//!
//! An asset that is missing, is not one or cannot be decoded is reported once, as a warning ([`MeshAssets::warnings`]
//! and the log), and never retried: its parts draw nothing (no mesh), draw untextured (no texture) or draw as plastic
//! (no material). Nothing about it can stop the game.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use metallic_project::material::Material;
use metallic_project::{AssetError, AssetReader, ImageAsset, MeshAsset};
use metallic_render::parts::{MeshData, MeshVertex};
use metallic_render::{InstanceScene, MaterialParams, Renderer};

/// What the worker is asked to load: a mesh, a texture fitted to the device's largest size, or a material.
enum Job {
    Mesh(Arc<str>),
    Texture(Arc<str>, u32),
    Material(Arc<str>),
}

/// What the worker loaded.
enum Loaded {
    Mesh(Arc<str>, Result<MeshData, AssetError>),
    Texture(Arc<str>, Result<ImageAsset, AssetError>),
    /// A material's parameters, with what was skipped in its file.
    Material(Arc<str>, Result<(MaterialParams, Vec<String>), AssetError>),
}

/// The renderer's parameters for `material`: its colours in linear light, emission times its strength.
pub fn material_params(material: &Material) -> MaterialParams {
    let linear = |c: [f32; 3]| {
        let v = metallic_core::Color3::new(c[0], c[1], c[2]).to_linear();
        [v.x, v.y, v.z]
    };
    MaterialParams {
        roughness: material.roughness,
        metalness: material.metallic,
        emissive: material.emission_strength,
        glass: material.glass,
        tint: linear(material.color),
        emission: linear(material.emission),
    }
}

/// A project's mesh and texture assets, loaded on a worker thread for the parts that name them. See the
/// [module documentation](self).
pub struct MeshAssets {
    /// `None` once the worker could not start.
    jobs: Option<Sender<Job>>,
    loaded: Receiver<Loaded>,
    worker: Option<JoinHandle<()>>,
    /// Every mesh and texture asked for, loaded or not.
    meshes: HashSet<Arc<str>>,
    textures: HashSet<Arc<str>>,
    /// Every material key asked for, loaded or not.
    materials: HashSet<Arc<str>>,
    /// The assets that could not be loaded, never asked for again.
    failed: HashSet<Arc<str>>,
    /// Asked for and not back yet.
    pending: usize,
    /// The scene's mesh-asset revision last looked at.
    seen: Option<u64>,
    /// The scene's material revision last looked at.
    seen_materials: Option<u64>,
    warnings: Vec<String>,
}

impl MeshAssets {
    /// Loads the assets `reader` reads (a project's: [`metallic_project::Project::asset_reader`]) on a worker thread.
    pub fn new(reader: AssetReader) -> Self {
        let (jobs, inbox) = mpsc::channel::<Job>();
        let (results, loaded) = mpsc::channel::<Loaded>();
        let mut reader = reader;
        let worker = std::thread::Builder::new()
            .name("metallic assets".into())
            .spawn(move || {
                while let Ok(job) = inbox.recv() {
                    if results.send(load(&mut reader, job)).is_err() {
                        break;
                    }
                }
            });
        let mut assets = Self {
            jobs: None,
            loaded,
            worker: None,
            meshes: HashSet::new(),
            textures: HashSet::new(),
            materials: HashSet::new(),
            failed: HashSet::new(),
            pending: 0,
            seen: None,
            seen_materials: None,
            warnings: Vec::new(),
        };
        match worker {
            Ok(worker) => {
                assets.jobs = Some(jobs);
                assets.worker = Some(worker);
            }
            Err(error) => assets.warn(format!("no asset loader thread could start ({error}); no assets load")),
        }
        assets
    }

    /// Asks for the assets `scene`'s mesh parts name that were not asked for yet (only when its mesh-asset kinds
    /// changed), and hands what the worker loaded since the last call to `renderer`. Returns how many assets reached
    /// the renderer. Waits for nothing; a frame with nothing new does no more than a check and an empty receive.
    pub fn update(&mut self, scene: &InstanceScene, renderer: &mut Renderer) -> usize {
        if self.seen != Some(scene.mesh_asset_revision()) {
            self.seen = Some(scene.mesh_asset_revision());
            self.request(scene, renderer.max_texture_size());
        }
        if self.seen_materials != Some(scene.material_revision()) {
            self.seen_materials = Some(scene.material_revision());
            self.request_materials(scene);
        }
        let mut added = 0;
        while let Ok(loaded) = self.loaded.try_recv() {
            added += self.hand_over(loaded, renderer);
        }
        added
    }

    /// Like [`MeshAssets::update`], then waits until every asset asked for is back and handed over, or `timeout`
    /// passed: for tests, tools and loading screens, never a game's frame. Returns whether all are back.
    pub fn wait(&mut self, scene: &InstanceScene, renderer: &mut Renderer, timeout: Duration) -> bool {
        self.update(scene, renderer);
        let deadline = Instant::now() + timeout;
        while self.pending > 0 {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.loaded.recv_timeout(left) {
                Ok(loaded) => {
                    self.hand_over(loaded, renderer);
                }
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return false,
            }
        }
        true
    }

    /// How many assets were asked for and are not back yet.
    pub fn pending(&self) -> usize {
        self.pending
    }

    /// What went wrong so far, one line per asset that could not be loaded.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Asks for every asset again at the next [`MeshAssets::update`]: for a new renderer, which has none of them.
    /// Assets that could not be loaded are not retried.
    pub fn reload(&mut self) {
        self.meshes.clear();
        self.textures.clear();
        self.materials.clear();
        self.seen = None;
        self.seen_materials = None;
    }

    /// Reads the material `key` names again at the next [`MeshAssets::update`], from `reader` (#678): for a material
    /// file that changed, or a key whose file appeared. A key that could not be loaded before is tried again.
    pub fn reload_material(&mut self, key: &str) {
        self.materials.remove(key);
        self.failed.remove(key);
        self.seen_materials = None;
    }

    /// Asks for every material of `scene` not asked for yet.
    fn request_materials(&mut self, scene: &InstanceScene) {
        for key in scene.material_keys() {
            if !self.failed.contains(key) && self.materials.insert(Arc::clone(key)) {
                self.send(Job::Material(Arc::clone(key)));
            }
        }
    }

    /// Asks for every mesh and texture of `scene`'s kinds not asked for yet.
    fn request(&mut self, scene: &InstanceScene, max_texture: u32) {
        for key in scene.mesh_assets() {
            if !self.failed.contains(&key.mesh) && self.meshes.insert(Arc::clone(&key.mesh)) {
                self.send(Job::Mesh(Arc::clone(&key.mesh)));
            }
            if !key.texture.is_empty()
                && !self.failed.contains(&key.texture)
                && self.textures.insert(Arc::clone(&key.texture))
            {
                self.send(Job::Texture(Arc::clone(&key.texture), max_texture));
            }
        }
    }

    fn send(&mut self, job: Job) {
        match self.jobs.as_ref().map(|jobs| jobs.send(job)) {
            Some(Ok(())) => self.pending += 1,
            // The worker never started (reported then) or stopped: nothing loads.
            None => {}
            Some(Err(mpsc::SendError(Job::Mesh(key) | Job::Texture(key, _) | Job::Material(key)))) => {
                self.failed.insert(Arc::clone(&key));
                self.warn(format!("asset {key}: the asset loader has stopped"));
            }
        }
    }

    /// Hands a loaded asset to `renderer`, or reports why it could not be loaded. Returns 1 when it reached the
    /// renderer.
    fn hand_over(&mut self, loaded: Loaded, renderer: &mut Renderer) -> usize {
        self.pending = self.pending.saturating_sub(1);
        let (key, added) = match loaded {
            Loaded::Mesh(key, Ok(mesh)) => {
                let added = renderer.add_mesh_asset(&key, &mesh).map_err(|error| error.to_string());
                (key, added.map_err(|error| format!("mesh: {error}")))
            }
            Loaded::Mesh(key, Err(error)) => (Arc::clone(&key), Err(format!("mesh {key}: {error}"))),
            Loaded::Texture(key, Ok(image)) => {
                let added = renderer
                    .add_texture_asset(&key, image.width, image.height, image.rgba)
                    .map_err(|error| error.to_string());
                (key, added.map_err(|error| format!("texture: {error}")))
            }
            Loaded::Texture(key, Err(error)) => (Arc::clone(&key), Err(format!("texture {key}: {error}"))),
            Loaded::Material(key, Ok((params, skipped))) => {
                renderer.parts_mut().materials_mut().set_material(&key, params);
                for skip in skipped {
                    self.warn(format!("material {key}: {skip}"));
                }
                (key, Ok(()))
            }
            // Its parts draw as the built-in material of that name, or plastic.
            Loaded::Material(key, Err(error)) => (Arc::clone(&key), Err(format!("material {key}: {error}"))),
        };
        match added {
            Ok(()) => 1,
            Err(warning) => {
                self.failed.insert(key);
                self.warn(warning);
                0
            }
        }
    }

    fn warn(&mut self, warning: String) {
        log::warn!("assets: {warning}");
        self.warnings.push(warning);
    }
}

impl Drop for MeshAssets {
    fn drop(&mut self) {
        // Closing the job channel ends the worker's loop; it finishes the asset it is on.
        self.jobs = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl std::fmt::Debug for MeshAssets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MeshAssets")
            .field("meshes", &self.meshes.len())
            .field("textures", &self.textures.len())
            .field("pending", &self.pending)
            .field("warnings", &self.warnings.len())
            .finish_non_exhaustive()
    }
}

/// Reads and decodes one asset (on the worker).
fn load(reader: &mut AssetReader, job: Job) -> Loaded {
    match job {
        Job::Mesh(key) => {
            let mesh = reader.load_mesh(&key).map(mesh_data);
            Loaded::Mesh(key, mesh)
        }
        Job::Texture(key, max) => {
            let image = reader.load_image(&key).map(|mut image| {
                image.fit(max);
                image
            });
            Loaded::Texture(key, image)
        }
        Job::Material(key) => {
            let material = reader
                .load_material(&key)
                .map(|loaded| (material_params(&loaded.material), loaded.warnings));
            Loaded::Material(key, material)
        }
    }
}

/// A decoded mesh as the renderer's vertices and indices.
fn mesh_data(mesh: MeshAsset) -> MeshData {
    let vertices = mesh
        .positions
        .iter()
        .zip(&mesh.normals)
        .zip(&mesh.uvs)
        .map(|((&position, &normal), &uv)| MeshVertex { position, normal, uv })
        .collect();
    MeshData {
        vertices,
        indices: mesh.indices,
    }
}
