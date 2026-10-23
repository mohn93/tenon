//! Styles: the model's list of named edge, background and fog settings, how deleting one
//! reaches the views that chose it, and how they are kept in files.

use kernel::{
    EdgeColorMode, Fog, FogColor, Ground, KernelError, Page, PageView, Scene, SceneData, Sketchy,
    Style, StyleId, StyleSettings,
};

fn view(style: Option<StyleId>) -> PageView {
    PageView {
        eye: [1.0, -10.0, 5.0],
        target: [0.0, 0.0, 0.0],
        up: [0.0, 0.0, 1.0],
        fov_y: 35f64.to_radians(),
        parallel: false,
        night: false,
        color_by_tag: false,
        x_ray: false,
        hidden_geometry: false,
        shadows: kernel::Shadows::default(),
        style,
    }
}

fn style(name: &str) -> Style {
    Style {
        name: name.into(),
        settings: StyleSettings::default(),
    }
}

fn names(s: &Scene) -> Vec<String> {
    s.styles().map(|(_, st)| st.name.clone()).collect()
}

#[test]
fn styles_are_added_with_ids_from_the_shared_counter_and_undone() {
    let mut s = Scene::new();
    let (a, change) = s.add_style(style("Hidden line")).unwrap();
    assert_eq!(a.to_string(), format!("y{}", a.0));
    assert_eq!(change.summary().styles, vec![a]);
    let (b, _) = s.add_style(style("Sketchy")).unwrap();
    assert!(b.0 > a.0);
    assert_eq!(names(&s), ["Hidden line", "Sketchy"]);
    assert_eq!(s.style(a).unwrap().settings, StyleSettings::default());
    s.validate().unwrap();
    s.apply(&change.inverse());
    assert!(s.style(a).is_none());
    assert_eq!(names(&s), ["Sketchy"]);
}

#[test]
fn style_names_are_trimmed_valid_and_distinct() {
    let mut s = Scene::new();
    let (a, _) = s.add_style(style("  Plain  ")).unwrap();
    assert_eq!(s.style(a).unwrap().name, "Plain");
    for bad in ["", "   ", &"x".repeat(101)] {
        let e = s.add_style(style(bad)).unwrap_err();
        assert!(matches!(e, KernelError::Style(_)), "{bad:?}: {e:?}");
    }
    assert!(matches!(
        s.add_style(style("Plain")),
        Err(KernelError::Style(_))
    ));
    assert!(matches!(
        s.add_style(style(" Plain ")),
        Err(KernelError::Style(_))
    ));
    assert!(s.add_style(style(&"x".repeat(100))).is_ok());
    assert_eq!(s.free_style_name(), "Style 1");
    s.add_style(style("Style 1")).unwrap();
    assert_eq!(s.free_style_name(), "Style 2");
}

#[test]
fn a_style_changes_as_one_step_and_keeps_its_own_name_free() {
    let mut s = Scene::new();
    let (a, _) = s.add_style(style("A")).unwrap();
    let (b, _) = s.add_style(style("B")).unwrap();
    let mut settings = StyleSettings::default();
    settings.edges.profiles = Some(4.0);
    settings.sky = Some([10, 20, 30]);
    let change = s
        .change_style(
            a,
            Style {
                name: "A".into(),
                settings,
            },
        )
        .unwrap();
    assert_eq!(change.steps.len(), 1);
    assert_eq!(change.summary().styles, vec![a]);
    assert_eq!(s.style(a).unwrap().settings.edges.profiles, Some(4.0));
    s.apply(&change.inverse());
    assert_eq!(s.style(a).unwrap().settings, StyleSettings::default());
    // Nothing to change changes nothing.
    assert!(s.change_style(a, style("A")).unwrap().is_empty());
    // A taken name is refused, and nothing changed.
    assert!(matches!(
        s.change_style(a, style("B")),
        Err(KernelError::Style(_))
    ));
    assert_eq!(s.style(a).unwrap().name, "A");
    assert_eq!(
        s.change_style(StyleId(999), style("Z")),
        Err(KernelError::StyleNotFound)
    );
    let _ = b;
}

#[test]
fn settings_ranges_are_checked() {
    let mut s = Scene::new();
    let with = |f: &dyn Fn(&mut StyleSettings)| {
        let mut st = StyleSettings::default();
        f(&mut st);
        Style {
            name: "S".into(),
            settings: st,
        }
    };
    let bad: Vec<Style> = vec![
        with(&|s| s.edges.profiles = Some(1.9)),
        with(&|s| s.edges.profiles = Some(10.1)),
        with(&|s| s.edges.profiles = Some(f32::NAN)),
        with(&|s| {
            s.edges.sketchy = Some(Sketchy {
                jitter: 10.5,
                extension: 0.0,
            })
        }),
        with(&|s| {
            s.edges.sketchy = Some(Sketchy {
                jitter: 1.0,
                extension: 30.5,
            })
        }),
        with(&|s| {
            s.edges.sketchy = Some(Sketchy {
                jitter: -1.0,
                extension: 0.0,
            })
        }),
        with(&|s| s.edges.endpoints = Some(1.0)),
        with(&|s| s.edges.endpoints = Some(31.0)),
        with(&|s| {
            s.ground = Some(Ground {
                color: [0; 3],
                opacity: 1.5,
                from_below: false,
            })
        }),
        with(&|s| {
            s.ground = Some(Ground {
                color: [0; 3],
                opacity: f32::INFINITY,
                from_below: false,
            })
        }),
        with(&|s| {
            s.fog = Some(Fog {
                start: 10.0,
                end: 10.0,
                color: FogColor::Background,
            })
        }),
        with(&|s| {
            s.fog = Some(Fog {
                start: -1.0,
                end: 10.0,
                color: FogColor::Background,
            })
        }),
        with(&|s| {
            s.fog = Some(Fog {
                start: 0.0,
                end: f32::INFINITY,
                color: FogColor::Background,
            })
        }),
    ];
    for st in bad {
        let e = s.add_style(st.clone()).unwrap_err();
        assert!(matches!(e, KernelError::Style(_)), "{st:?}: {e:?}");
    }
    let good = with(&|s| {
        s.edges.color_mode = EdgeColorMode::ByAxis;
        s.edges.profiles = Some(2.0);
        s.edges.sketchy = Some(Sketchy {
            jitter: 10.0,
            extension: 30.0,
        });
        s.edges.endpoints = Some(30.0);
        s.edges.show_edges = false;
        s.ground = Some(Ground {
            color: [1, 2, 3],
            opacity: 1.0,
            from_below: true,
        });
        s.fog = Some(Fog {
            start: 0.0,
            end: 100.0,
            color: FogColor::Rgb([9, 9, 9]),
        });
    });
    s.add_style(good).unwrap();
}

#[test]
fn deleting_a_style_drops_it_from_the_scenes_that_chose_it_in_one_undoable_change() {
    let mut s = Scene::new();
    let (a, _) = s.add_style(style("A")).unwrap();
    let (b, _) = s.add_style(style("B")).unwrap();
    let on = |name: &str, st| Page {
        name: name.into(),
        view: Some(view(st)),
        tags: None,
    };
    let (p1, _) = s.add_page(on("one", Some(a))).unwrap();
    let (p2, _) = s.add_page(on("two", Some(b))).unwrap();
    let (p3, _) = s.add_page(on("three", None)).unwrap();
    let before = s.clone();
    let change = s.delete_style(a).unwrap();
    assert!(s.style(a).is_none());
    assert_eq!(s.page(p1).unwrap().view.unwrap().style, None);
    assert_eq!(s.page(p2).unwrap().view.unwrap().style, Some(b));
    assert_eq!(s.page(p3).unwrap().view.unwrap().style, None);
    assert_eq!(change.summary().styles, vec![a]);
    assert_eq!(change.summary().scenes, vec![p1]);
    s.validate().unwrap();
    s.apply(&change.inverse());
    assert_eq!(s, before);
    assert_eq!(s.page(p1).unwrap().view.unwrap().style, Some(a));
    assert_eq!(
        s.delete_style(StyleId(999)),
        Err(KernelError::StyleNotFound)
    );
}

#[test]
fn a_page_cannot_choose_a_missing_style() {
    let mut s = Scene::new();
    let page = Page {
        name: "p".into(),
        view: Some(view(Some(StyleId(77)))),
        tags: None,
    };
    assert!(matches!(
        s.add_page(page),
        Err(KernelError::Page(_) | KernelError::StyleNotFound)
    ));
}

#[test]
fn styles_round_trip_through_data_and_a_dangling_choice_is_refused() {
    let mut s = Scene::new();
    let mut settings = StyleSettings::default();
    settings.edges.profiles = Some(3.5);
    settings.background = [4, 5, 6];
    settings.fog = Some(Fog {
        start: 1.0,
        end: 50.0,
        color: FogColor::Background,
    });
    let (a, _) = s
        .add_style(Style {
            name: "Fog".into(),
            settings,
        })
        .unwrap();
    s.add_page(Page {
        name: "p".into(),
        view: Some(view(Some(a))),
        tags: None,
    })
    .unwrap();
    let data = s.to_data();
    let text = serde_json::to_string(&data).unwrap();
    let read: SceneData = serde_json::from_str(&text).unwrap();
    let back = Scene::from_data(&read).unwrap();
    assert_eq!(back, s);
    // Absent without styles: older output is unchanged.
    let none = Scene::new().to_data();
    assert!(!serde_json::to_string(&none).unwrap().contains("styles"));
    // A page choosing a style the model lacks is refused, as are repeated names and ids.
    let mut dangling = data.clone();
    dangling.styles.clear();
    assert!(Scene::from_data(&dangling).is_err());
    let mut twice = data.clone();
    twice.styles.push(twice.styles[0].clone());
    assert!(Scene::from_data(&twice).is_err());
    let mut same_name = data.clone();
    let mut other = same_name.styles[0].clone();
    other.id += 1000;
    same_name.styles.push(other);
    assert!(Scene::from_data(&same_name).is_err());
    let mut bad_range = data.clone();
    bad_range.styles[0].settings.edges.profiles = Some(99.0);
    assert!(Scene::from_data(&bad_range).is_err());
}

#[test]
fn a_view_that_chooses_no_style_writes_no_style_field() {
    let v = serde_json::to_value(view(None)).unwrap();
    assert!(v.get("style").is_none());
    let v = serde_json::to_value(view(Some(StyleId(5)))).unwrap();
    assert_eq!(v["style"], "y5");
}

#[test]
fn style_ids_parse() {
    assert_eq!("y5".parse::<StyleId>(), Ok(StyleId(5)));
    assert_eq!(StyleId(5).to_string(), "y5");
    for s in ["", "y", "s5", "y05", "y+5"] {
        assert!(s.parse::<StyleId>().is_err(), "{s:?} parsed");
    }
}
