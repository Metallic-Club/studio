//! Shared math types across the runtime's direct dependency and CPU animation/asset APIs.

use glam::{Mat4, Quat, Vec3};
use metallic_anim::{Transform as AnimationTransform, glam as animation_math};
use metallic_asset::Transform as AssetTransform;

#[test]
fn runtime_animation_and_asset_math_types_are_directly_compatible() {
    // Explicit types make public type compatibility part of the test.
    let translation: animation_math::Vec3 = Vec3::new(3.0, -2.0, 5.0);
    let rotation: animation_math::Quat = Quat::from_xyzw(0.5, 0.5, 0.5, 0.5);
    let scale = Vec3::new(2.0, 3.0, 4.0);
    let animation = AnimationTransform {
        translation,
        rotation,
        scale,
    };
    let asset = AssetTransform {
        translation: animation.translation,
        rotation: animation.rotation,
        scale: animation.scale,
    };
    let animation_matrix: Mat4 = animation.to_mat4();
    let asset_matrix: animation_math::Mat4 = asset.to_mat4();
    assert_eq!(animation_matrix, asset_matrix);

    // This unit quaternion cycles (x, y, z) to (z, x, y) with exact dyadic arithmetic.
    // Nonuniform scale followed by that rotation and translation maps (1, 2, 3) to (15, 0, 11).
    let point = Vec3::new(1.0, 2.0, 3.0);
    let expected = Vec3::new(15.0, 0.0, 11.0);
    assert_eq!(animation.transform_point(point), expected);
    assert_eq!(asset_matrix.transform_point3(point), expected);

    let offset: animation_math::Mat4 = Mat4::from_translation(Vec3::new(-1.0, 2.0, 0.5));
    let composed: Mat4 = offset * asset.to_mat4();
    assert_eq!(composed.transform_point3(point), Vec3::new(14.0, 2.0, 11.5));
}
