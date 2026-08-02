#!/usr/bin/env python3
"""Patch the pinned wgpu tree with a deterministic core presentation experiment.

This script is an execution carrier for Fieldwork issue teamleaderleo/fieldwork#116.
It deliberately modifies the noop HAL only in the CI workspace, then adds one
wgpu-core unit test. The committed source branch remains a research carrier,
not a proposed upstream implementation.
"""

from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f"{label}: expected one match, found {count}")
    return text.replace(old, new, 1)


noop_path = Path("wgpu-hal/src/noop/mod.rs")
noop = noop_path.read_text()

noop = replace_once(
    noop,
    "use core::{ptr, sync::atomic::Ordering, time::Duration};",
    "use core::{\n    ptr,\n    sync::atomic::{AtomicUsize, Ordering},\n    time::Duration,\n};",
    "noop atomic import",
)

noop = replace_once(
    noop,
    "type DeviceResult<T> = Result<T, crate::DeviceError>;\n",
    """type DeviceResult<T> = Result<T, crate::DeviceError>;

// Fieldwork execution counters. These are injected only in the CI workspace.
static FIELDWORK_SURFACE_ACQUIRES: AtomicUsize = AtomicUsize::new(0);
static FIELDWORK_SURFACE_DISCARDS: AtomicUsize = AtomicUsize::new(0);
static FIELDWORK_SURFACE_PRESENTS: AtomicUsize = AtomicUsize::new(0);
static FIELDWORK_SURFACE_OUTSTANDING: AtomicUsize = AtomicUsize::new(0);

pub fn fieldwork_reset_surface_counters() {
    FIELDWORK_SURFACE_ACQUIRES.store(0, Ordering::SeqCst);
    FIELDWORK_SURFACE_DISCARDS.store(0, Ordering::SeqCst);
    FIELDWORK_SURFACE_PRESENTS.store(0, Ordering::SeqCst);
    FIELDWORK_SURFACE_OUTSTANDING.store(0, Ordering::SeqCst);
}

pub fn fieldwork_surface_counters() -> (usize, usize, usize, usize) {
    (
        FIELDWORK_SURFACE_ACQUIRES.load(Ordering::SeqCst),
        FIELDWORK_SURFACE_DISCARDS.load(Ordering::SeqCst),
        FIELDWORK_SURFACE_PRESENTS.load(Ordering::SeqCst),
        FIELDWORK_SURFACE_OUTSTANDING.load(Ordering::SeqCst),
    )
}
""",
    "noop counter insertion",
)

noop = replace_once(
    noop,
    """    unsafe fn surface_capabilities(&self, surface: &Context) -> Option<crate::SurfaceCapabilities> {
        None
    }
""",
    """    unsafe fn surface_capabilities(
        &self,
        _surface: &Context,
    ) -> Option<crate::SurfaceCapabilities> {
        Some(crate::SurfaceCapabilities {
            formats: vec![wgt::SurfaceFormatCapabilities {
                format: wgt::TextureFormat::Rgba8UnormSrgb,
                color_spaces: wgt::SurfaceColorSpaces::SRGB,
            }],
            maximum_frame_latency: 1..=3,
            current_extent: None,
            usage: wgt::TextureUses::COLOR_TARGET,
            present_modes: vec![wgt::PresentMode::Fifo],
            composite_alpha_modes: vec![wgt::CompositeAlphaMode::Opaque],
        })
    }
""",
    "noop surface capabilities",
)

noop = replace_once(
    noop,
    """    unsafe fn acquire_texture(
        &self,
        _timeout: Option<Duration>,
        _fence: &Fence,
    ) -> Result<crate::AcquiredSurfaceTexture<Api>, crate::SurfaceError> {
        Err(crate::SurfaceError::Timeout)
    }
    unsafe fn discard_texture(&self, texture: Resource) {}
""",
    """    unsafe fn acquire_texture(
        &self,
        _timeout: Option<Duration>,
        _fence: &Fence,
    ) -> Result<crate::AcquiredSurfaceTexture<Api>, crate::SurfaceError> {
        FIELDWORK_SURFACE_ACQUIRES.fetch_add(1, Ordering::SeqCst);
        FIELDWORK_SURFACE_OUTSTANDING.fetch_add(1, Ordering::SeqCst);
        Ok(crate::AcquiredSurfaceTexture {
            texture: Resource,
            suboptimal: false,
        })
    }

    unsafe fn discard_texture(&self, _texture: Resource) {
        FIELDWORK_SURFACE_DISCARDS.fetch_add(1, Ordering::SeqCst);
        FIELDWORK_SURFACE_OUTSTANDING.fetch_sub(1, Ordering::SeqCst);
    }
""",
    "noop acquire and discard",
)

noop = replace_once(
    noop,
    """    unsafe fn present(
        &self,
        surface: &Context,
        texture: Resource,
    ) -> Result<(), crate::SurfaceError> {
        Ok(())
    }
""",
    """    unsafe fn present(
        &self,
        _surface: &Context,
        _texture: Resource,
    ) -> Result<(), crate::SurfaceError> {
        FIELDWORK_SURFACE_PRESENTS.fetch_add(1, Ordering::SeqCst);
        FIELDWORK_SURFACE_OUTSTANDING.fetch_sub(1, Ordering::SeqCst);
        Ok(())
    }
""",
    "noop present",
)

noop_path.write_text(noop)

present_path = Path("wgpu-core/src/present.rs")
present = present_path.read_text()

present = replace_once(
    present,
    "const FRAME_TIMEOUT_MS: u32 = 1000;\n",
    """const FRAME_TIMEOUT_MS: u32 = 1000;

#[cfg(test)]
static FIELDWORK_FAIL_AFTER_DETACH: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);
""",
    "core failpoint declaration",
)

present = replace_once(
    present,
    """        };

        // If the texture was never rendered to, clear it and transition to
""",
    """        };

        #[cfg(test)]
        if FIELDWORK_FAIL_AFTER_DETACH.swap(false, core::sync::atomic::Ordering::SeqCst) {
            return Err(SurfaceError::Device(DeviceError::Lost));
        }

        // If the texture was never rendered to, clear it and transition to
""",
    "core post-detach failpoint",
)

present += r'''

#[cfg(all(test, feature = "noop", target_os = "linux"))]
mod fieldwork_pre_hal_tests {
    use super::*;
    use crate::{device::DeviceDescriptor, instance::Instance};
    use alloc::sync::Arc;
    use raw_window_handle::{
        RawDisplayHandle, RawWindowHandle, XlibDisplayHandle, XlibWindowHandle,
    };

    fn configured_noop_surface() -> (Arc<Surface>, Arc<Queue>) {
        let mut descriptor = wgt::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgt::Backends::NOOP;
        descriptor.backend_options.noop = wgt::NoopBackendOptions::enabled();
        let instance = Instance::new("fieldwork-pre-hal-probe", descriptor, None);

        let display = RawDisplayHandle::Xlib(XlibDisplayHandle::new(None, 0));
        let window = RawWindowHandle::Xlib(XlibWindowHandle::new(1));
        let surface = unsafe { instance.create_surface(Some(display), window) }
            .expect("noop surface creation should succeed");

        let adapter = instance
            .request_adapter(
                &wgt::RequestAdapterOptions {
                    compatible_surface: Some(surface.as_ref()),
                    ..Default::default()
                },
                wgt::Backends::NOOP,
            )
            .expect("instrumented noop adapter should support the surface");

        let (device, queue) = adapter
            .request_device(&DeviceDescriptor::default())
            .expect("noop device creation should succeed");

        let config = wgt::SurfaceConfiguration {
            usage: wgt::TextureUsages::RENDER_ATTACHMENT,
            format: wgt::TextureFormat::Rgba8UnormSrgb,
            width: 4,
            height: 4,
            present_mode: wgt::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgt::CompositeAlphaMode::Opaque,
            view_formats: Vec::new(),
            color_space: wgt::SurfaceColorSpace::Srgb,
        };
        assert_eq!(surface.configure(&device, &config), None);

        (surface, queue)
    }

    #[test]
    fn fieldwork_pre_hal_failure_detaches_without_discard() {
        hal::noop::fieldwork_reset_surface_counters();
        let (surface, queue) = configured_noop_surface();

        let first = surface
            .get_current_texture_inner()
            .expect("first controlled acquisition should return");
        assert_eq!(first.status, wgt::SurfaceStatus::Good);
        assert!(first.texture.is_some());
        assert_eq!(hal::noop::fieldwork_surface_counters(), (1, 0, 0, 1));

        FIELDWORK_FAIL_AFTER_DETACH.store(true, core::sync::atomic::Ordering::SeqCst);
        let failure = queue.present(&surface).expect_err("failpoint must stop before HAL present");
        assert!(matches!(failure, SurfaceError::Device(DeviceError::Lost)));
        assert!(
            surface
                .presentation
                .lock()
                .as_ref()
                .expect("surface should remain configured")
                .acquired_texture
                .is_none(),
            "the core surface record is detached before the injected failure"
        );
        assert_eq!(
            hal::noop::fieldwork_surface_counters(),
            (1, 0, 0, 1),
            "neither HAL present nor surface discard returned the first image"
        );
        drop(first);

        let second = surface
            .get_current_texture_inner()
            .expect("core allows another acquisition after detachment");
        assert_eq!(second.status, wgt::SurfaceStatus::Good);
        assert!(second.texture.is_some());
        assert_eq!(
            hal::noop::fieldwork_surface_counters(),
            (2, 0, 0, 2),
            "the controlled backend now has two outstanding images"
        );

        surface
            .discard_inner()
            .expect("discard should return the second image");
        drop(second);
        assert_eq!(
            hal::noop::fieldwork_surface_counters(),
            (2, 1, 0, 1),
            "discarding the second image leaves the first detached image outstanding"
        );

        // Negative control: the ordinary successful path balances acquisition and present.
        hal::noop::fieldwork_reset_surface_counters();
        let (control_surface, control_queue) = configured_noop_surface();
        let control = control_surface
            .get_current_texture_inner()
            .expect("control acquisition should return");
        assert!(control.texture.is_some());
        control_queue
            .present(&control_surface)
            .expect("control presentation should succeed");
        drop(control);
        assert_eq!(
            hal::noop::fieldwork_surface_counters(),
            (1, 0, 1, 0),
            "successful presentation must balance the controlled ownership counter"
        );
    }
}
'''

present_path.write_text(present)
