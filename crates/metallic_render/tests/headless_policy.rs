//! Error/selection policy without creating an adapter or opening a window.
#[path = "support/gpu.rs"]
mod gpu;

use metallic_render::RenderError;

#[test]
fn setup_only_skips_explicit_adapter_absence() {
    assert_eq!(gpu::context_result(Ok(17)), Some(17));
    assert_eq!(
        gpu::context_result::<()>(Err(RenderError::NoAdapter("unavailable".into()))),
        None
    );
    for error in [
        RenderError::Device("device refused".into()),
        RenderError::Readback("mapping failed".into()),
        RenderError::InvalidSize,
        RenderError::Surface("unexpected surface error".into()),
    ] {
        let outcome = std::panic::catch_unwind(|| gpu::context_result::<()>(Err(error)));
        assert!(outcome.is_err(), "a setup failure became a successful skip");
    }
}

#[test]
fn capture_failures_cannot_become_skips() {
    for error in [
        RenderError::Readback("mapping failed".into()),
        RenderError::Device("device lost".into()),
        RenderError::NoAdapter("unexpected during capture".into()),
        RenderError::InvalidSize,
    ] {
        let outcome = std::panic::catch_unwind(|| gpu::require_result::<()>("capture", Err(error)));
        assert!(outcome.is_err());
    }
    assert_eq!(gpu::require_result("capture", Ok(vec![1, 2, 3])), [1, 2, 3]);
}

#[test]
fn test_backend_selection_is_explicit_and_strict() {
    assert_eq!(gpu::parse_backend(None).unwrap(), None);
    assert_eq!(gpu::parse_backend(Some("default")).unwrap(), None);
    assert_eq!(
        gpu::parse_backend(Some("dx12")).unwrap(),
        Some(wgpu::Backends::DX12)
    );
    assert_eq!(
        gpu::parse_backend(Some("vulkan")).unwrap(),
        Some(wgpu::Backends::VULKAN)
    );
    assert_eq!(
        gpu::parse_backend(Some("gl")).unwrap(),
        Some(wgpu::Backends::GL)
    );
    for invalid in ["", "DX12", "vulakn", "dx12,vulkan"] {
        assert!(gpu::parse_backend(Some(invalid)).is_err());
    }
}
