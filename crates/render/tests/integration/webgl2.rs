//! Every pipeline variant under WebGL2's downlevel limits, as the web build's GL adapter gives
//! them: a uniform, binding or vertex attribute added past them fails here, not in a browser.
//! This checks the limits, not the GLSL translation.

use glam::Vec3;
use kernel::{EdgeStyle, Fog, FogColor, Ground, Sketchy, StyleSettings};
use render::{Renderer, SunLight};
use session::Lighting;

use crate::golden::{add_box, doc, look, run};

#[test]
fn every_pipeline_builds_under_webgl2_limits() {
    let instance = wgpu::Instance::default();
    let Ok(adapter) =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
    else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let limits = wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits());
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: limits,
        ..Default::default()
    }))
    .unwrap();
    let errors = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let e2 = errors.clone();
    device.on_uncaptured_error(std::sync::Arc::new(move |e| {
        e2.lock().unwrap().push(e.to_string());
    }));
    let mut d = doc();
    add_box(&mut d, [0.0, 0.0]);
    // An edge up the front face splits it in two coplanar faces, so the indexed candidate draw
    // of profiles and endpoints runs too.
    run(
        &mut d,
        session::Command::AddEdge {
            start: [1.0, 0.0, 0.0],
            end: [1.0, 0.0, 1.0],
        },
    );
    look(&mut d, [6.0, -8.0, 5.0], [1.0, 0.75, 0.5]);
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut r = Renderer::without_shadows(&device, &queue, format);
    let style = StyleSettings {
        sky: Some([40, 80, 220]),
        ground: Some(Ground {
            color: [60, 160, 60],
            opacity: 1.0,
            from_below: false,
        }),
        fog: Some(Fog {
            start: 1.0,
            end: 30.0,
            color: FogColor::Background,
        }),
        edges: EdgeStyle {
            profiles: Some(4.0),
            sketchy: Some(Sketchy {
                jitter: 3.0,
                extension: 5.0,
            }),
            endpoints: Some(6.0),
            ..EdgeStyle::default()
        },
        ..StyleSettings::default()
    };
    let (w, h) = (320, 240);
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = tex.create_view(&Default::default());
    let mut cam = d.camera.clone();
    cam.viewport = glam::DVec2::new(w as f64, h as f64);
    for (styled, sun) in [(false, false), (true, false), (false, true), (true, true)] {
        r.set_model_style(styled.then_some(&style));
        r.set_sun(sun.then_some(SunLight {
            direction: Vec3::new(-0.5, -0.3, 0.8).normalize(),
            light: 0.8,
            dark: 0.45,
            on_ground: true,
        }));
        r.set_model(d.scene(), &[]);
        r.set_selection(d.selection());
        r.set_lighting(Lighting::Day);
        r.render(&view, (w, h), &cam, &[]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    }
    let errs = errors.lock().unwrap();
    assert!(errs.is_empty(), "{errs:#?}");
}
