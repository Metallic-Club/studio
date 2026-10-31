//! Frozen files emitted by main a0bb1bf with glam 0.30.10, before the shared-math migration.
use metallic_anim::format::{clip_from_text, clip_to_text, skeleton_from_text, skeleton_to_text};
use metallic_anim::glam::{Quat, Vec3};
use metallic_anim::{Channel, JointChannel, TrackTarget};

const CLIP: &str = include_str!("data/clip-glam-0.30.ron");
const SKELETON: &str = include_str!("data/skeleton-glam-0.30.ron");

#[test]
fn pre_migration_clip_preserves_vector_and_quaternion_values_and_text() {
    let clip = clip_from_text(CLIP).unwrap();
    assert_eq!(clip.name, "Legacy math");
    assert_eq!(clip.duration, 1.0);
    let translation = clip
        .track(&TrackTarget::joint("Root", JointChannel::Translation))
        .unwrap();
    let Channel::Vec3(curve) = &translation.channel else {
        panic!("legacy translation must remain a Vec3 channel");
    };
    assert_eq!(curve.keys()[0].value, Vec3::new(1.25, -2.5, 7.75));
    assert_eq!(curve.keys()[1].value, Vec3::new(3.0, 4.0, -1.0));
    let rotation = clip
        .track(&TrackTarget::joint("Tip", JointChannel::Rotation))
        .unwrap();
    let Channel::Quat(curve) = &rotation.channel else {
        panic!("legacy rotation must remain a quaternion channel");
    };
    assert_eq!(curve.keys()[0].value, Quat::from_xyzw(0.5, 0.5, 0.5, 0.5));
    assert_eq!(curve.keys()[1].value, Quat::from_xyzw(0.0, 0.0, 1.0, 0.0));
    // Git may translate text line endings on checkout; serialized field/component order must not change.
    assert_eq!(
        clip_to_text(&clip).unwrap().replace("\r\n", "\n"),
        CLIP.replace("\r\n", "\n")
    );
}

#[test]
fn pre_migration_skeleton_preserves_hierarchy_nonuniform_scale_and_text() {
    let skeleton = skeleton_from_text(SKELETON).unwrap();
    assert_eq!(skeleton.len(), 2);
    assert_eq!(skeleton.find("Root"), Some(0));
    assert_eq!(skeleton.find("Tip"), Some(1));
    assert_eq!(skeleton.parent(1), Some(0));
    let root = &skeleton.joints()[0];
    assert_eq!(root.rest.translation, Vec3::new(1.25, -2.5, 7.75));
    assert_eq!(root.rest.rotation, Quat::from_xyzw(0.5, 0.5, 0.5, 0.5));
    assert_eq!(root.rest.scale, Vec3::new(0.5, 2.0, 4.0));
    assert_eq!(root.length, 3.5);
    let tip = &skeleton.joints()[1];
    assert_eq!(tip.rest.translation, Vec3::new(2.0, -1.0, 0.25));
    assert_eq!(tip.rest.rotation, Quat::from_xyzw(0.0, 0.0, 1.0, 0.0));
    assert_eq!(tip.rest.scale, Vec3::ONE);
    assert_eq!(tip.length, 0.75);
    assert_eq!(
        skeleton_to_text(&skeleton).unwrap().replace("\r\n", "\n"),
        SKELETON.replace("\r\n", "\n")
    );
}
