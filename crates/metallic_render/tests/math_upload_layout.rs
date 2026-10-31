//! The shared math version must preserve the renderer's column-major GPU upload contract.
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
        material: [0.5, 0.0, 3.0, 0.0],
    };
    assert_eq!(std::mem::size_of::<InstanceRaw>(), 96);
    assert_eq!(std::mem::offset_of!(InstanceRaw, model), 0);
    assert_eq!(std::mem::offset_of!(InstanceRaw, color), 64);
    assert_eq!(std::mem::offset_of!(InstanceRaw, material), 80);
    let floats: &[f32] = bytemuck::cast_slice(bytemuck::bytes_of(&instance));
    assert_eq!(&floats[..16], columns.as_flattened());
    assert_eq!(&floats[16..20], &instance.color);
    assert_eq!(&floats[20..], &instance.material);
}
