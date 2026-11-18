//! Small pictures of the project's images, decoded off the UI thread (#771).
//!
//! A thumbnail of a picture was decoded on the UI thread, at full size, two a frame, each from a new asset reader:
//! a folder of large pictures, or the picture window's grid, stalled Studio for as long as they took. Now a field,
//! tile or row asks for one ([`PictureLoader::want`]) and draws its icon meanwhile; a worker thread reads and decodes
//! the picture, shrinks it to at most [`SIDE`] pixels and sends it back, and the UI thread turns a few a frame into
//! textures ([`PictureLoader::receive`]). Each picture is asked for once; one that cannot be read is remembered as
//! such. The worker reads with an [`AssetReader`] of the project made at most once a frame, when there is something
//! new to read.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

use metallic_engine::metallic_project::{AssetId, AssetReader, Project};

/// The largest side of a thumbnail, in pixels.
pub const SIDE: u32 = 64;
/// How many decoded pictures become textures in one frame.
const UPLOADS_A_FRAME: usize = 8;

/// What the worker is asked to do.
enum Job {
    /// Read from now on with this reader: a newer view of the project.
    Reader(AssetReader),
    /// Decode the picture `id`.
    Decode(AssetId),
}

/// A decoded, shrunk picture: its size and its pixels (RGBA, not premultiplied).
struct Small {
    size: [usize; 2],
    rgba: Vec<u8>,
}

/// The thumbnails' worker, started on the first picture asked for.
#[derive(Default)]
pub struct PictureLoader {
    jobs: Option<Sender<Job>>,
    done: Option<Receiver<(AssetId, Option<Small>)>>,
    /// Asked for and not back yet.
    asked: HashSet<AssetId>,
    /// The frame the worker's reader was made in.
    reader_pass: Option<u64>,
    /// The worker could not start: nothing is decoded.
    failed: bool,
}

impl std::fmt::Debug for PictureLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PictureLoader")
            .field("asked", &self.asked.len())
            .field("failed", &self.failed)
            .finish_non_exhaustive()
    }
}

impl PictureLoader {
    /// Asks for the thumbnail of `project`'s picture `id`, unless it was asked for already. It arrives through
    /// [`PictureLoader::receive`] on a later frame, which `ctx` is asked to draw.
    pub fn want(&mut self, ctx: &egui::Context, project: &Project, id: AssetId) {
        if self.asked.contains(&id) || self.failed {
            return;
        }
        let Some(jobs) = self.start(ctx) else {
            return;
        };
        let pass = ctx.cumulative_pass_nr();
        let mut sent = true;
        if self.reader_pass != Some(pass) {
            sent = jobs.send(Job::Reader(project.asset_reader())).is_ok();
            self.reader_pass = Some(pass);
        }
        if sent && jobs.send(Job::Decode(id)).is_ok() {
            self.asked.insert(id);
        } else {
            log::warn!("thumbnails: the worker stopped");
            self.failed = true;
        }
    }

    /// Whether the picture `id` was asked for and is not back yet.
    pub fn is_pending(&self, id: AssetId) -> bool {
        self.asked.contains(&id)
    }

    /// Takes the thumbnails decoded since the last call, a few at most, into `cache` as textures: `None` for a
    /// picture that could not be read. Those `pixelated` says are sampled nearest (#781), so they show crisp.
    pub fn receive(
        &mut self,
        ctx: &egui::Context,
        cache: &mut HashMap<AssetId, Option<egui::TextureHandle>>,
        pixelated: impl Fn(AssetId) -> bool,
    ) {
        let Some(done) = &self.done else {
            return;
        };
        for _ in 0..UPLOADS_A_FRAME {
            match done.try_recv() {
                Ok((id, small)) => {
                    self.asked.remove(&id);
                    let texture = small.map(|small| {
                        let image = egui::ColorImage::from_rgba_unmultiplied(small.size, &small.rgba);
                        let options = if pixelated(id) {
                            egui::TextureOptions::NEAREST
                        } else {
                            egui::TextureOptions::LINEAR
                        };
                        ctx.load_texture(format!("metallic.thumbnail.{id}"), image, options)
                    });
                    cache.insert(id, texture);
                }
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.failed = true;
                    return;
                }
            }
        }
        // More are waiting: the next frame takes them.
        ctx.request_repaint();
    }

    /// The worker's queue, starting the worker the first time.
    fn start(&mut self, ctx: &egui::Context) -> Option<Sender<Job>> {
        if let Some(jobs) = &self.jobs {
            return Some(jobs.clone());
        }
        let (jobs, inbox) = mpsc::channel::<Job>();
        let (outbox, done) = mpsc::channel();
        let ctx = ctx.clone();
        let started = std::thread::Builder::new()
            .name("thumbnails".into())
            .spawn(move || work(&inbox, &outbox, &ctx));
        if let Err(error) = started {
            log::warn!("thumbnails: could not start the worker: {error}");
            self.failed = true;
            return None;
        }
        self.jobs = Some(jobs.clone());
        self.done = Some(done);
        Some(jobs)
    }
}

/// The worker: decodes each picture asked for with the latest reader, until Studio lets go of the queue.
fn work(inbox: &Receiver<Job>, outbox: &Sender<(AssetId, Option<Small>)>, ctx: &egui::Context) {
    let mut reader: Option<AssetReader> = None;
    while let Ok(job) = inbox.recv() {
        match job {
            Job::Reader(newer) => reader = Some(newer),
            Job::Decode(id) => {
                let small = reader.as_mut().and_then(|reader| decode(reader, id));
                if outbox.send((id, small)).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
        }
    }
}

/// The picture `id`, shrunk to at most [`SIDE`] pixels; `None` when it cannot be read.
fn decode(reader: &mut AssetReader, id: AssetId) -> Option<Small> {
    let mut image = reader.load_image(&id.to_string()).ok()?;
    image.fit(SIDE);
    Some(Small {
        size: [image.width as usize, image.height as usize],
        rgba: image.rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2×1 PNG: red then blue.
    fn png() -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[255, 0, 0, 255, 0, 0, 255, 255]).unwrap();
        writer.finish().unwrap();
        bytes
    }

    #[test]
    fn pictures_are_decoded_off_the_ui_thread_once_each() {
        let dir = std::env::temp_dir().join(format!("metallic-picture-loader-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let template = metallic_engine::metallic_project::Template::from_dir(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/baseplate-3d"),
        )
        .unwrap();
        let mut project = Project::create(dir.join("Pictures.mtproj"), &template, "Pictures").unwrap();
        project.write("Assets/dot.png", png()).unwrap();
        project.write("Assets/broken.png", vec![1, 2, 3]).unwrap();
        project.refresh_assets().unwrap();
        let dot = project.assets().id("Assets/dot.png").unwrap();
        let broken = project.assets().id("Assets/broken.png").unwrap();

        let ctx = egui::Context::default();
        let mut loader = PictureLoader::default();
        let mut cache = HashMap::new();
        loader.want(&ctx, &project, dot);
        loader.want(&ctx, &project, dot);
        loader.want(&ctx, &project, broken);
        assert!(loader.is_pending(dot) && loader.is_pending(broken));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while cache.len() < 2 && std::time::Instant::now() < deadline {
            loader.receive(&ctx, &mut cache, |_| false);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let texture = cache.get(&dot).cloned().flatten().expect("decoded");
        assert_eq!(texture.size(), [2, 1]);
        assert!(matches!(cache.get(&broken), Some(None)), "a failure is remembered");
        assert!(!loader.is_pending(dot));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
