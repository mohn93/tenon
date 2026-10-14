use std::sync::Arc;

use glam::{DAffine3, DVec2, DVec3};
use kernel::{
    DefinitionKind, ImageFormat, KernelError, MAX_TEXTURES, MeshId, Model, Scene, Texture,
    TextureImage, texture_axes, texture_position,
};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

/// A 2 x 1 pixel image: red, then blue.
fn image() -> Arc<TextureImage> {
    Arc::new(TextureImage {
        format: ImageFormat::Png,
        encoded: b"not decoded by the kernel".to_vec(),
        width: 2,
        height: 1,
        pixels: vec![255, 0, 0, 255, 0, 0, 255, 255],
    })
}

fn texture(width: f64, height: f64) -> Texture {
    Texture {
        image: image(),
        width,
        height,
    }
}

#[test]
fn textures_lie_along_the_horizontal_or_along_x_on_level_faces() {
    // A wall facing -y: u runs along +x, v up.
    let (u, v) = texture_axes(-DVec3::Y);
    assert!(u.abs_diff_eq(DVec3::X, 1e-12) && v.abs_diff_eq(DVec3::Z, 1e-12));
    // A wall facing +x: u along +y.
    let (u, v) = texture_axes(DVec3::X);
    assert!(u.abs_diff_eq(DVec3::Y, 1e-12) && v.abs_diff_eq(DVec3::Z, 1e-12));
    // Level faces: u along x, v along y on top, along -y underneath.
    let (u, v) = texture_axes(DVec3::Z);
    assert!(u.abs_diff_eq(DVec3::X, 1e-12) && v.abs_diff_eq(DVec3::Y, 1e-12));
    let (u, v) = texture_axes(-DVec3::Z);
    assert!(u.abs_diff_eq(DVec3::X, 1e-12) && v.abs_diff_eq(-DVec3::Y, 1e-12));
    // A roof sloping up toward +y: u stays horizontal, v climbs the slope.
    let n = p(0.0, -1.0, 1.0).normalize();
    let (u, v) = texture_axes(n);
    assert!(u.abs_diff_eq(DVec3::X, 1e-12));
    assert!(v.abs_diff_eq(p(0.0, 1.0, 1.0).normalize(), 1e-12));
    assert!(texture_position(-DVec3::Y, p(2.0, 5.0, 3.0)).abs_diff_eq(DVec2::new(2.0, 3.0), 1e-12));
}

#[test]
fn a_material_takes_a_texture_and_undo_restores_the_old_one() {
    let mut s = Scene::from_root(Model::new());
    let (id, _) = s
        .add_material_with_texture("Brick", [200, 0, 0], 1.0, Some(texture(1.0, 0.5)))
        .unwrap();
    assert_eq!(s.texture_count(), 1);
    assert_eq!(s.material(id).unwrap().texture, Some(texture(1.0, 0.5)));

    let bigger = s
        .change_material_with_texture(id, None, None, None, Some(Some(texture(2.0, 1.0))))
        .unwrap();
    assert_eq!(s.material(id).unwrap().texture, Some(texture(2.0, 1.0)));
    s.apply(&bigger.inverse());
    assert_eq!(s.material(id).unwrap().texture, Some(texture(1.0, 0.5)));

    // Changing only the name keeps the texture; removing it keeps the color.
    s.change_material(id, Some("Old brick"), None, None)
        .unwrap();
    assert!(s.material(id).unwrap().texture.is_some());
    let removed = s
        .change_material_with_texture(id, None, None, None, Some(None))
        .unwrap();
    let m = s.material(id).unwrap();
    assert_eq!((m.texture.clone(), m.color), (None, [200, 0, 0]));
    s.apply(&removed.inverse());
    assert_eq!(s.material(id).unwrap().texture, Some(texture(1.0, 0.5)));
    s.validate().unwrap();
}

#[test]
fn texture_sizes_and_counts_are_limited() {
    let mut s = Scene::from_root(Model::new());
    for bad in [0.0, 0.0005, -1.0, 1001.0, f64::NAN, f64::INFINITY] {
        let err = s
            .add_material_with_texture("Bad", [0, 0, 0], 1.0, Some(texture(bad, 1.0)))
            .unwrap_err();
        assert!(
            matches!(err, KernelError::MaterialTexture(_)),
            "{bad}: {err:?}"
        );
    }
    let broken = Texture {
        image: Arc::new(TextureImage {
            pixels: vec![0; 4],
            ..(*image()).clone()
        }),
        width: 1.0,
        height: 1.0,
    };
    assert!(matches!(
        s.add_material_with_texture("Broken", [0, 0, 0], 1.0, Some(broken)),
        Err(KernelError::MaterialTexture(_))
    ));
    assert_eq!(s.materials().count(), 0);

    for k in 0..MAX_TEXTURES {
        s.add_material_with_texture(&format!("T{k}"), [0, 0, 0], 1.0, Some(texture(1.0, 1.0)))
            .unwrap();
    }
    let err = s
        .add_material_with_texture("One more", [0, 0, 0], 1.0, Some(texture(1.0, 1.0)))
        .unwrap_err();
    assert!(err.to_string().contains("at most 256 textures"), "{err}");
    // A material that has one can still change it.
    let (first, _) = s.materials().next().unwrap();
    s.change_material_with_texture(first, None, None, None, Some(Some(texture(3.0, 3.0))))
        .unwrap();
    // One without cannot take one.
    let (plain, _) = s.add_material("Plain", [1, 2, 3]).unwrap();
    assert!(
        s.change_material_with_texture(plain, None, None, None, Some(Some(texture(1.0, 1.0))))
            .is_err()
    );
}

#[test]
fn textures_reach_files_with_their_images() {
    let mut s = Scene::from_root(Model::new());
    let (id, _) = s
        .add_material_with_texture("Brick", [200, 0, 0], 0.5, Some(texture(1.0, 0.5)))
        .unwrap();
    let mut data = s.to_data();
    let t = data.materials[0].texture.as_ref().unwrap();
    assert_eq!(t.file, format!("textures/m{}.png", id.0));
    assert_eq!((t.width, t.height), (1.0, 0.5));
    assert_eq!(t.image.as_deref(), Some(&*image()));
    // The image is not written: a reader puts it back.
    let json = serde_json::to_value(&data).unwrap();
    assert_eq!(
        json["materials"][0]["texture"],
        serde_json::json!({"file": t.file, "width": 1.0, "height": 0.5})
    );
    assert_eq!(Scene::from_data(&data).unwrap(), s);
    data.materials[0].texture.as_mut().unwrap().image = None;
    let err = Scene::from_data(&data).unwrap_err();
    assert!(err.contains("texture has no image"), "{err}");
    data.materials[0].texture.as_mut().unwrap().image = Some(image());
    data.materials[0].texture.as_mut().unwrap().width = -2.0;
    assert!(Scene::from_data(&data).is_err());
}

#[test]
fn triangles_carry_texture_positions_in_their_definition() {
    // A 1 m wall square in the xz plane (the kernel turns its front to +y), made a component
    // and placed 10 m along x, turned a quarter about z.
    let mut m = Model::new();
    m.add_polygon(&[
        p(0.0, 0.0, 0.0),
        p(1.0, 0.0, 0.0),
        p(1.0, 0.0, 1.0),
        p(0.0, 0.0, 1.0),
    ])
    .unwrap();
    let mut s = Scene::from_root(m);
    let edges: Vec<_> = s.root().edges().map(|(id, _)| id).collect();
    let faces: Vec<_> = s.root().faces().map(|(id, _)| id).collect();
    let (instance, _) = s
        .make_group(
            MeshId::Root,
            &edges,
            &faces,
            &[],
            DefinitionKind::Component,
            "Panel",
        )
        .unwrap();
    let turn = DAffine3::from_translation(p(10.0, 0.0, 0.0))
        * DAffine3::from_rotation_z(std::f64::consts::FRAC_PI_2);
    s.edit(MeshId::Root, |m| {
        Ok(m.transform_instances(&[instance], turn))
    })
    .unwrap();
    let tris = s.triangles();
    assert_eq!(tris.len(), 2);
    for t in &tris {
        let normal = (t.corners[1] - t.corners[0])
            .cross(t.corners[2] - t.corners[0])
            .normalize();
        assert!(normal.abs_diff_eq(t.normal, 1e-9));
        assert!(t.normal.abs_diff_eq(-DVec3::X, 1e-9));
        for (c, uv) in t.corners.iter().zip(t.uvs) {
            // A definition point (x, 0, z) lands at (10, x, z). Its front faces +y there, so
            // the texture's u runs along -x and v up, whatever the placement.
            let def_x = c.y;
            assert!(uv.abs_diff_eq(DVec2::new(-def_x, c.z), 1e-9), "{c} {uv}");
        }
    }
}
