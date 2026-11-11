//! Shading views (#478): debug views of how the parts are drawn, for Studio's viewport (and any tool). The renderer
//! draws [`ShadingView::Shaded`] unless told otherwise ([`crate::Renderer::set_shading_view`]).
//!
//! Every view but [`ShadingView::Wireframe`] is a branch of the parts' fragment shader on the frame uniform
//! (`view` in frame.wgsl): no pipeline changes. The wireframe draws the parts' triangles as lines with pipelines of
//! its own, compiled the first time the view is chosen, on devices with line drawing
//! ([`crate::Capabilities::polygon_mode_line`]); until they exist, or where they cannot, the parts are drawn shaded.

/// How the parts are drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ShadingView {
    /// As the game draws them: lit, shadowed, with their materials.
    #[default]
    Shaded,
    /// Their own colours (and textures), with no light or shadow.
    Unlit,
    /// The light and shadow alone, on a plain grey surface of each part's material.
    LightingOnly,
    /// A heat map of how many point and spot lights reach each pixel: dark blue none, then blue, cyan, green,
    /// yellow, orange and red for 8 or more, over each surface's sunlit shape.
    LightDensity,
    /// The parts' triangles as lines, each in its part's colour.
    Wireframe,
}

impl ShadingView {
    /// Every view, in the order a menu lists them.
    pub const ALL: [ShadingView; 5] = [
        ShadingView::Shaded,
        ShadingView::Unlit,
        ShadingView::LightingOnly,
        ShadingView::LightDensity,
        ShadingView::Wireframe,
    ];

    /// Its name, as a menu shows it.
    pub fn name(self) -> &'static str {
        match self {
            ShadingView::Shaded => "Shaded",
            ShadingView::Unlit => "Unlit",
            ShadingView::LightingOnly => "Lighting only",
            ShadingView::LightDensity => "Light density",
            ShadingView::Wireframe => "Wireframe",
        }
    }

    /// Its name in a settings file.
    pub fn as_str(self) -> &'static str {
        match self {
            ShadingView::Shaded => "shaded",
            ShadingView::Unlit => "unlit",
            ShadingView::LightingOnly => "lighting_only",
            ShadingView::LightDensity => "light_density",
            ShadingView::Wireframe => "wireframe",
        }
    }

    /// The view a settings file names, if it names one.
    pub fn parse(name: &str) -> Option<ShadingView> {
        ShadingView::ALL.into_iter().find(|view| view.as_str() == name)
    }

    /// Its number in the frame uniform (`view.x` in frame.wgsl).
    pub(crate) fn number(self) -> f32 {
        match self {
            ShadingView::Shaded => 0.0,
            ShadingView::Unlit => 1.0,
            ShadingView::LightingOnly => 2.0,
            ShadingView::LightDensity => 3.0,
            ShadingView::Wireframe => 4.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_view_reads_back_as_it_was_written() {
        for view in ShadingView::ALL {
            assert_eq!(ShadingView::parse(view.as_str()), Some(view));
        }
        assert_eq!(ShadingView::parse("x-ray"), None);
        assert_eq!(ShadingView::default(), ShadingView::Shaded);
    }
}
