use miniquad::{KeyMods, UniformType};
use std::{io::Cursor, sync::Arc};
use darkfi_serial::Decodable;
use freetype as ft;

use crate::{error::{Error, Result}, prop::Property, scene::{SceneGraph, SceneNodeId, Pimpl, Slot}, gfx::{Rectangle, RenderContext, COLOR_WHITE, COLOR_BLUE, COLOR_RED, COLOR_GREEN}, text::TextShaper};

pub type EditBoxPtr = Arc<EditBox>;

pub struct EditBox {
    scroll: Arc<Property>,
}

impl EditBox {
    pub fn new(scene_graph: &mut SceneGraph, node_id: SceneNodeId) -> Result<Pimpl> {
        let node = scene_graph.get_node(node_id).unwrap();
        let scroll = node.get_property("scroll").ok_or(Error::PropertyNotFound)?;

        println!("EditBox::new()");
        let self_ = Arc::new(Self{
            scroll
        });
        let weak_self = Arc::downgrade(&self_);

        let slot = Slot {
            name: "editbox::key_down".to_string(),
            func: Box::new(move |data| {
                let mut cur = Cursor::new(&data);
                let keymods = KeyMods {
                    shift: Decodable::decode(&mut cur).unwrap(),
                    ctrl: Decodable::decode(&mut cur).unwrap(),
                    alt: Decodable::decode(&mut cur).unwrap(),
                    logo: Decodable::decode(&mut cur).unwrap(),
                };
                let repeat = bool::decode(&mut cur).unwrap();
                let key = String::decode(&mut cur).unwrap();

                let self_ = weak_self.upgrade();
                if let Some(self_) = self_ {
                    self_.key_press(key, keymods, repeat);
                }
            }),
        };

        let keyb_node = 
            scene_graph
            .lookup_node_mut("/window/input/keyboard")
            .expect("no keyboard attached!");
        keyb_node.register("key_down", slot);

        // Save any properties we use
        Ok(Pimpl::EditBox(self_))
    }

    pub fn render<'a>(&self, render: &mut RenderContext<'a>, node_id: SceneNodeId, layer_rect: &Rectangle<i32>) -> Result<()> {
        let node = render.scene_graph.get_node(node_id).unwrap();

        let text = node.get_property_str("text")?;
        let font_size = node.get_property_f32("font_size")?;
        let debug = node.get_property_bool("debug")?;
        let rect = RenderContext::get_dim(node, layer_rect)?;
        let baseline = node.get_property_f32("baseline")?;
        let scroll = node.get_property_f32("scroll")?;
        let cursor_pos = node.get_property_u32("cursor_pos")?;

        let color_prop = node.get_property("color").ok_or(Error::PropertyNotFound)?;
        let color_r = color_prop.get_f32(0)?;
        let color_g = color_prop.get_f32(1)?;
        let color_b = color_prop.get_f32(2)?;
        let color_a = color_prop.get_f32(3)?;
        let text_color = [color_r, color_g, color_b, color_a];

        let layer_w = layer_rect.w as f32;
        let layer_h = layer_rect.h as f32;
        let off_x = rect.x / layer_w;
        let off_y = rect.y / layer_h;
        // Use absolute pixel scale
        let scale_x = 1. / layer_w;
        let scale_y = 1. / layer_h;
        //let model = glam::Mat4::IDENTITY;
        let model = glam::Mat4::from_translation(glam::Vec3::new(off_x, off_y, 0.)) *
            glam::Mat4::from_scale(glam::Vec3::new(scale_x, scale_y, 1.));

        let mut uniforms_data = [0u8; 128];
        let data: [u8; 64] = unsafe { std::mem::transmute_copy(&render.proj) };
        uniforms_data[0..64].copy_from_slice(&data);
        let data: [u8; 64] = unsafe { std::mem::transmute_copy(&model) };
        uniforms_data[64..].copy_from_slice(&data);
        assert_eq!(128, 2 * UniformType::Mat4.size());

        render.ctx.apply_uniforms_from_bytes(uniforms_data.as_ptr(), uniforms_data.len());

        let shaper = TextShaper {
            font_faces: render.font_faces
        };

        let mut glyph_idx = 0;
        let mut rhs = 0.;

        for glyph in shaper.shape(text, font_size, text_color) {
            let texture = render.ctx.new_texture_from_rgba8(glyph.bmp_width, glyph.bmp_height, &glyph.bmp);

            let x1 = glyph.pos.x + scroll;
            let y1 = glyph.pos.y + baseline;
            let x2 = x1 + glyph.pos.w;
            let y2 = y1 + glyph.pos.h;

            let bound = Rectangle {
                x: 0.,
                y: 0.,
                w: rect.w,
                h: rect.h,
            };
            render.render_clipped_box_with_texture(&bound, x1, y1, x2, y2, COLOR_WHITE, texture);
            //render.render_box_with_texture(x1, y1, x2, y2, COLOR_WHITE, texture);
            render.ctx.delete_texture(texture);

            if debug {
                render.outline(x1, y1, x2, y2, COLOR_BLUE, 1.);
            }

            if cursor_pos == 0 {
                let cursor_color = [1., 0.5, 0.5, 1.];
                render.render_box(0., 0., 4., rect.h, cursor_color);
            }
            else if cursor_pos == glyph_idx + 1 {
                let cursor_color = [1., 0.5, 0.5, 1.];
                render.render_box(x2, 0., x2 + 4., rect.h, cursor_color);
            }

            glyph_idx += 1;

            rhs = x2;
        }
        if debug {
            render.hline(0., rhs, 0., COLOR_RED, 1.);
            render.outline(0., 0., rect.w, rect.h, COLOR_GREEN, 1.);
        }

        Ok(())
    }

    fn key_press(self: Arc<Self>, key: String, mods: KeyMods, repeat: bool) {
        if repeat {
            return;
        }
        if key == "PageUp" {
            println!("pageup!");
        }
        else if key == "PageDown" {
            println!("pagedown!");
        }
        // Ability to move cursor
    }
}

