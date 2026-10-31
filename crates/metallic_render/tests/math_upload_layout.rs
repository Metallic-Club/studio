//! The shared math version must preserve the renderer's column-major GPU upload contract.
use metallic_render::MaterialTable;
use metallic_render::glam::{Mat4, Quat, Vec3};
use metallic_render::scene::InstanceRaw;

#[test]
fn nonuniform_rotated_transform_keeps_column_order_and_instance_offsets() {
    // Exact binary fractions rotate X->Y, Y->Z and Z->X without a trigonometric tolerance.
    let model = Mat4::from_scale_rotation_translation(
        Vec3::new(2.0, 3.0, 4.0),
        Quat::from_xyzw(0.5, 0.5, 0.5, 0.5),
        Vec3::new(5.0, 6.0, 7.0),
    );
    let columns = [
        [0.0, 2.0, 0.0, 0.0],
        [0.0, 0.0, 3.0, 0.0],
        [4.0, 0.0, 0.0, 0.0],
        [5.0, 6.0, 7.0, 1.0],
    ];
    assert_eq!(model.to_cols_array_2d(), columns);
    assert_eq!(model.transform_point3(Vec3::ONE), Vec3::new(9.0, 8.0, 10.0));
    // Exercises glam's bytemuck opt-in, not only conversion into array fields.
    assert_eq!(bytemuck::bytes_of(&model), bytemuck::bytes_of(&columns));
    let instance = InstanceRaw {
        model: model.to_cols_array_2d(),
        color: [0.25, 0.5, 0.75, 1.0],
        material: [MaterialTable::row(0, 3), 0, 3, 0],
    };
    assert_eq!(std::mem::size_of::<InstanceRaw>(), 96);
    assert_eq!(std::mem::offset_of!(InstanceRaw, model), 0);
    assert_eq!(std::mem::offset_of!(InstanceRaw, color), 64);
    assert_eq!(std::mem::offset_of!(InstanceRaw, material), 80);
    let bytes = bytemuck::bytes_of(&instance);
    let floats: &[f32] = bytemuck::cast_slice(&bytes[..80]);
    assert_eq!(&floats[..16], columns.as_flattened());
    assert_eq!(&floats[16..], &instance.color);
    // The material-table ABI is integer words, not roughness/metalness floats.
    let material_words: &[u32] = bytemuck::cast_slice(&bytes[80..]);
    assert_eq!(material_words, &[3, 0, 3, 0]);
    assert_eq!(material_words, &instance.material);
}

#[test]
fn replacement_camera_constructors_keep_right_handed_y_up_zero_to_one_depth() {
    use metallic_render::glam::camera::rh::{proj::directx, view};
    let eye = Vec3::new(2.0, 3.0, 4.0);
    let target = eye - Vec3::Z;
    let view = view::look_at_mat4(eye, target, Vec3::Y);
    assert_eq!(view.transform_point3(eye), Vec3::ZERO);
    assert_eq!(view.transform_point3(target), Vec3::NEG_Z);
    assert_eq!(view.transform_vector3(Vec3::Y), Vec3::Y);
    let perspective = directx::perspective(std::f32::consts::FRAC_PI_2, 2.0, 1.0, 2.0);
    let near = perspective * Vec3::new(0.0, 0.0, -1.0).extend(1.0);
    let far = perspective * Vec3::new(0.0, 0.0, -2.0).extend(1.0);
    assert_eq!(near.z / near.w, 0.0);
    assert_eq!(far.z / far.w, 1.0);
    assert!((perspective * Vec3::new(0.0, 1.0, -1.0).extend(1.0)).y > 0.0);
    let ortho = directx::orthographic(-2.0, 2.0, -4.0, 4.0, 0.0, 8.0);
    assert_eq!(ortho.transform_point3(Vec3::ZERO).z, 0.0);
    assert_eq!(ortho.transform_point3(Vec3::new(0.0, 0.0, -8.0)).z, 1.0);
    assert_eq!(
        ortho.transform_point3(Vec3::new(2.0, 4.0, 0.0)),
        Vec3::new(1.0, 1.0, 0.0)
    );
}
