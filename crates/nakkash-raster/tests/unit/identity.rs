use std::sync::Arc;

use super::*;
use crate::blend::BlendMode;
use crate::effect::{Effect, ParamValue};
use crate::list::{MaskMode, MatteMode};
use crate::picture::Picture;
use crate::style::FillRule;
use vello_cpu::kurbo::Shape;

fn square(color: Color) -> Node {
    Node::Path(PathDraw {
        path: Rect::new(0.0, 0.0, 4.0, 4.0).to_path(0.1),
        paint: Paint::Solid(color),
        style: DrawStyle::Fill(FillRule::NonZero),
    })
}

fn id(node: &Node) -> Id {
    Identities::default().node(node)
}

/// A layer with something in every field.
fn full() -> Layer {
    let mut layer = Layer::new(vec![square(Color::WHITE)]);
    layer.transform = Affine::translate((3.0, 4.0));
    layer.opacity = 0.5;
    layer.blend = BlendMode::Multiply;
    layer.clip = Some(Clip::Rect(Rect::new(0.0, 0.0, 9.0, 9.0)));
    layer.masks = vec![LayerMask {
        path: Rect::new(1.0, 1.0, 3.0, 3.0).to_path(0.1),
        mode: MaskMode::Add,
        inverted: false,
        opacity: 1.0,
        feather: [0.0, 0.0],
        expand: 0.0,
    }];
    layer.effects = vec![
        Effect::new("gaussian-blur", &[("blurriness", ParamValue::Number(2.0))])
            .expect("known")
            .with_input(Arc::new(square(Color::WHITE))),
    ];
    layer.matte = Some(Matte {
        source: Box::new(square(Color::WHITE)),
        mode: MatteMode::Alpha,
    });
    layer.bounds = Some(Rect::new(0.0, 0.0, 4.0, 4.0));
    layer.time = 1.5;
    layer.motion = vec![Affine::IDENTITY];
    layer.projection = Some(Projection::new(Homography::IDENTITY));
    layer
}

#[test]
fn equal_nodes_have_one_identity_wherever_they_are() {
    let a = Node::Layer(full());
    let b = Node::Layer(full());
    assert_eq!(id(&a), id(&b));
    // A copy, in another place in memory, too.
    let boxed = Box::new(a.clone());
    assert_eq!(id(&boxed), id(&a));
    // Identities do not depend on the run: a fixed key.
    assert_eq!(id(&a), id(&Node::Layer(full())));
}

#[test]
fn a_change_to_any_field_changes_the_identity() {
    let base = id(&Node::Layer(full()));
    type Change = Box<dyn Fn(&mut Layer)>;
    let changes: Vec<(&str, Change)> = vec![
        (
            "transform",
            Box::new(|l| l.transform = Affine::translate((3.0, 5.0))),
        ),
        ("opacity", Box::new(|l| l.opacity = 0.25)),
        ("blend", Box::new(|l| l.blend = BlendMode::Screen)),
        ("clip", Box::new(|l| l.clip = None)),
        ("mask", Box::new(|l| l.masks[0].inverted = true)),
        (
            "mask feather",
            Box::new(|l| l.masks[0].feather = [0.0, 1.0]),
        ),
        (
            "effect value",
            Box::new(|l| {
                l.effects[0] =
                    Effect::new("gaussian-blur", &[("blurriness", ParamValue::Number(3.0))])
                        .expect("known")
                        .with_input(Arc::new(square(Color::WHITE)));
            }),
        ),
        (
            "effect input",
            Box::new(|l| {
                l.effects[0] =
                    Effect::new("gaussian-blur", &[("blurriness", ParamValue::Number(2.0))])
                        .expect("known")
                        .with_input(Arc::new(square(Color::rgba(1.0, 0.0, 0.0, 1.0))));
            }),
        ),
        (
            "children",
            Box::new(|l| l.children.push(square(Color::WHITE))),
        ),
        ("isolate", Box::new(|l| l.isolate = true)),
        (
            "matte mode",
            Box::new(|l| l.matte.as_mut().expect("matte").mode = MatteMode::Luma),
        ),
        (
            "matte source",
            Box::new(|l| {
                *l.matte.as_mut().expect("matte").source = square(Color::rgba(0.0, 0.0, 0.0, 1.0));
            }),
        ),
        ("adjust", Box::new(|l| l.adjust = true)),
        ("bounds", Box::new(|l| l.bounds = None)),
        ("motion", Box::new(|l| l.motion.push(Affine::scale(2.0)))),
        ("projection", Box::new(|l| l.projection = None)),
    ];
    for (field, change) in changes {
        let mut layer = full();
        change(&mut layer);
        assert_ne!(
            id(&Node::Layer(layer)),
            base,
            "{field} left the identity as it was"
        );
    }
    // A mark of sharing says nothing of what is drawn.
    let mut marked = full();
    marked.shared = Some(crate::list::Shared::new());
    assert_eq!(id(&Node::Layer(marked)), base);
}

#[test]
fn numbers_count_by_their_bits() {
    let at = |x: f64| {
        let mut layer = Layer::new(Vec::new());
        layer.transform = Affine::translate((x, 0.0));
        id(&Node::Layer(layer))
    };
    assert_ne!(at(0.0), at(-0.0));
    assert_eq!(at(f64::NAN), at(f64::NAN));
    assert_ne!(at(1.0), at(1.0 + f64::EPSILON));
}

#[test]
fn pictures_are_known_by_their_pixels() {
    let image = |v: u8| crate::Image::new(2, 2, vec![v; 16]).expect("an image");
    let draw = |picture: Picture| {
        Node::Image(ImageDraw {
            picture,
            rect: Rect::new(0.0, 0.0, 2.0, 2.0),
        })
    };
    let a = draw(Picture::from_image(&image(7)).expect("a picture"));
    let b = draw(Picture::from_image(&image(7)).expect("a picture"));
    let c = draw(Picture::from_image(&image(8)).expect("a picture"));
    assert_eq!(id(&a), id(&b), "two pictures of the same pixels");
    assert_ne!(id(&a), id(&c));
}

#[test]
fn kinds_of_nodes_with_the_same_numbers_differ() {
    let path = square(Color::WHITE);
    let in_layer = Node::Layer(Layer::new(vec![square(Color::WHITE)]));
    assert_ne!(id(&path), id(&in_layer));
    let mut ids = Identities::default();
    assert_ne!(ids.list(std::slice::from_ref(&path)), id(&path));
    assert_ne!(ids.list(&[]), ids.list(std::slice::from_ref(&path)));
}

#[test]
fn a_layers_time_counts_only_where_an_effect_reads_it() {
    let at = |time: f64, effect: &str| {
        let mut layer = Layer::new(vec![square(Color::WHITE)]);
        layer.effects = vec![Effect::new(effect, &[]).expect("known")];
        layer.time = time;
        id(&Node::Layer(layer))
    };
    assert_eq!(at(0.5, "gaussian-blur"), at(1.5, "gaussian-blur"));
    assert_ne!(at(0.5, "particles"), at(1.5, "particles"));
}

const FONT: &[u8] = include_bytes!("../fixtures/FixtureSans-Black.ttf");

/// A font over `bytes`, loaded as a blob of its own.
fn font(bytes: Vec<u8>, index: u32) -> vello_cpu::peniko::FontData {
    vello_cpu::peniko::FontData::new(vello_cpu::peniko::Blob::new(Arc::new(bytes)), index)
}

/// A glyph run of fixed glyphs and paint in `font`.
fn glyphs(font: vello_cpu::peniko::FontData) -> Node {
    Node::Glyphs(GlyphDraw {
        font,
        size: 12.0,
        glyphs: vec![crate::list::Glyph {
            id: 3,
            x: 1.0,
            y: 2.0,
        }],
        glyph_transform: None,
        coords: Vec::new(),
        paint: Paint::Solid(Color::BLACK),
        style: DrawStyle::Fill(FillRule::NonZero),
    })
}

/// The fixture font with a byte in its middle changed: other bytes, same size.
fn other_bytes() -> Vec<u8> {
    let mut bytes = FONT.to_vec();
    bytes[FONT.len() / 2] ^= 1;
    bytes
}

#[test]
fn fonts_are_known_by_their_bytes_not_by_their_load() {
    let a = glyphs(font(FONT.to_vec(), 0));
    let b = glyphs(font(FONT.to_vec(), 0));
    assert_eq!(id(&a), id(&b));
}

// The repository has one test font and no collection, so "another font" is
// the same font with a middle byte changed, and "another face" is index 1 of it
// (the index is hashed, never read).
#[rstest::rstest]
#[case::other_bytes(font(other_bytes(), 0))]
#[case::other_face(font(FONT.to_vec(), 1))]
fn another_font_or_face_is_another_identity(#[case] other: vello_cpu::peniko::FontData) {
    let first = glyphs(font(FONT.to_vec(), 0));
    assert_ne!(id(&first), id(&glyphs(other)));
}

#[test]
fn a_dropped_fonts_id_is_never_reused() {
    // Blob ids come from a process-wide counter that only goes up, so the
    // memo keyed by them cannot answer for another font.
    let a = font(FONT.to_vec(), 0);
    let first = id(&glyphs(a.clone()));
    let dropped = a.data.id();
    drop(a);
    let b = font(other_bytes(), 0);
    assert_ne!(b.data.id(), dropped);
    assert_ne!(id(&glyphs(b)), first);
}

#[test]
fn a_blob_given_another_blobs_id_is_still_its_own_font() {
    // `Blob::from_raw_parts` can attach any id to any bytes; the memo must
    // not answer with the first font's hash for the second.
    use vello_cpu::peniko::{Blob, FontData};
    let a = font(FONT.to_vec(), 0);
    let first = id(&glyphs(a.clone()));
    let forged = Blob::from_raw_parts(Arc::new(other_bytes()), a.data.id());
    assert_eq!(forged.id(), a.data.id());
    assert_ne!(id(&glyphs(FontData::new(forged, 0))), first);
}

#[test]
fn a_forged_id_of_a_dropped_font_is_still_its_own_font() {
    // The same, after the first blob is gone and its memory may be reused.
    use vello_cpu::peniko::{Blob, FontData};
    let a = font(FONT.to_vec(), 0);
    let first = id(&glyphs(a.clone()));
    let dropped = a.data.id();
    drop(a);
    let forged = Blob::from_raw_parts(Arc::new(other_bytes()), dropped);
    assert_ne!(id(&glyphs(FontData::new(forged, 0))), first);
}

/// A content hash is the identity hasher over raw bytes, with a fixed key:
/// the same value in every run and on every machine, so a record written
/// on one day is checked on another.
#[test]
fn a_content_hash_is_the_same_everywhere() {
    assert_eq!(
        content_hash("pixels", b"nakkash"),
        0xc80f_6d55_92c7_8766_8f1d_ecda_27f0_ec85
    );
}

/// The tag and the bytes are both written with their lengths, so moving a
/// byte from one to the other is another hash.
#[test]
fn a_content_hash_tells_tags_and_bytes_apart() {
    let hashes = [
        content_hash("pixels", b"ab"),
        content_hash("mask", b"ab"),
        content_hash("pixels", b"ac"),
        content_hash("pixels", b"abc"),
        content_hash("pixelsa", b"b"),
        content_hash("pixels", b""),
    ];
    for (i, a) in hashes.iter().enumerate() {
        for b in &hashes[i + 1..] {
            assert_ne!(a, b);
        }
    }
}
