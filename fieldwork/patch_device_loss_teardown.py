#!/usr/bin/env python3
"""Patch the pinned wgpu tree with a deterministic device-loss teardown experiment.

The patch is applied only in a GitHub Actions workspace. It instruments noop
surface-texture ownership, proves the current unconfigure-before-release order,
and tests whether releasing the retained core texture first satisfies the
unconfigure precondition.
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
    """#[derive(Debug)]
pub struct Resource;

#[derive(Debug)]
pub struct Fence {
""",
    """#[derive(Debug)]
pub struct Resource;

#[derive(Debug)]
pub struct SurfaceResource {
    texture: Resource,
    accounted: bool,
}

impl SurfaceResource {
    fn acquired() -> Self {
        Self {
            texture: Resource,
            accounted: false,
        }
    }
}

impl core::borrow::Borrow<Resource> for SurfaceResource {
    fn borrow(&self) -> &Resource {
        &self.texture
    }
}

impl Drop for SurfaceResource {
    fn drop(&mut self) {
        if !self.accounted {
            FIELDWORK_IMPLICIT_RELEASES.fetch_add(1, Ordering::SeqCst);
            let previous = FIELDWORK_OUTSTANDING.fetch_sub(1, Ordering::SeqCst);
            assert!(previous > 0, "implicit surface release underflow");
        }
    }
}

#[derive(Debug)]
pub struct Fence {
""",
    "surface resource insertion",
)

noop = replace_once(
    noop,
    "type DeviceResult<T> = Result<T, crate::DeviceError>;\n",
    """type DeviceResult<T> = Result<T, crate::DeviceError>;

static FIELDWORK_ACQUIRES: AtomicUsize = AtomicUsize::new(0);
static FIELDWORK_DISCARDS: AtomicUsize = AtomicUsize::new(0);
static FIELDWORK_PRESENTS: AtomicUsize = AtomicUsize::new(0);
static FIELDWORK_IMPLICIT_RELEASES: AtomicUsize = AtomicUsize::new(0);
static FIELDWORK_OUTSTANDING: AtomicUsize = AtomicUsize::new(0);
static FIELDWORK_UNCONFIGURES: AtomicUsize = AtomicUsize::new(0);
static FIELDWORK_UNCONFIGURE_VIOLATIONS: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldworkSurfaceCounters {
    pub acquires: usize,
    pub discards: usize,
    pub presents: usize,
    pub implicit_releases: usize,
    pub outstanding: usize,
    pub unconfigures: usize,
    pub unconfigure_violations: usize,
}

pub fn fieldwork_reset_surface_counters() {
    FIELDWORK_ACQUIRES.store(0, Ordering::SeqCst);
    FIELDWORK_DISCARDS.store(0, Ordering::SeqCst);
    FIELDWORK_PRESENTS.store(0, Ordering::SeqCst);
    FIELDWORK_IMPLICIT_RELEASES.store(0, Ordering::SeqCst);
    FIELDWORK_OUTSTANDING.store(0, Ordering::SeqCst);
    FIELDWORK_UNCONFIGURES.store(0, Ordering::SeqCst);
    FIELDWORK_UNCONFIGURE_VIOLATIONS.store(0, Ordering::SeqCst);
}

pub fn fieldwork_surface_counters() -> FieldworkSurfaceCounters {
    FieldworkSurfaceCounters {
        acquires: FIELDWORK_ACQUIRES.load(Ordering::SeqCst),
        discards: FIELDWORK_DISCARDS.load(Ordering::SeqCst),
        presents: FIELDWORK_PRESENTS.load(Ordering::SeqCst),
        implicit_releases: FIELDWORK_IMPLICIT_RELEASES.load(Ordering::SeqCst),
        outstanding: FIELDWORK_OUTSTANDING.load(Ordering::SeqCst),
        unconfigures: FIELDWORK_UNCONFIGURES.load(Ordering::SeqCst),
        unconfigure_violations: FIELDWORK_UNCONFIGURE_VIOLATIONS.load(Ordering::SeqCst),
    }
}
""",
    "counter insertion",
)

noop = replace_once(
    noop,
    "type SurfaceTexture = Resource;",
    "type SurfaceTexture = SurfaceResource;",
    "surface associated type",
)

noop = replace_once(
    noop,
    "crate::impl_dyn_resource!(Buffer, CommandBuffer, Context, Fence, Resource);",
    "crate::impl_dyn_resource!(Buffer, CommandBuffer, Context, Fence, Resource, SurfaceResource);",
    "dynamic surface resource registration",
)

noop = replace_once(
    noop,
    "impl crate::DynSurfaceTexture for Resource {}",
    "impl crate::DynSurfaceTexture for SurfaceResource {}",
    "surface dynamic trait",
)

noop = replace_once(
    noop,
    """    unsafe fn unconfigure(&self, device: &Context) {}

    unsafe fn acquire_texture(
        &self,
        _timeout: Option<Duration>,
        _fence: &Fence,
    ) -> Result<crate::AcquiredSurfaceTexture<Api>, crate::SurfaceError> {
        Err(crate::SurfaceError::Timeout)
    }
    unsafe fn discard_texture(&self, texture: Resource) {}
""",
    """    unsafe fn unconfigure(&self, _device: &Context) {
        FIELDWORK_UNCONFIGURES.fetch_add(1, Ordering::SeqCst);
        let outstanding = FIELDWORK_OUTSTANDING.load(Ordering::SeqCst);
        if outstanding != 0 {
            FIELDWORK_UNCONFIGURE_VIOLATIONS.fetch_add(1, Ordering::SeqCst);
            panic!(
                "instrumented unconfigure observed {outstanding} outstanding surface resource(s)"
            );
        }
    }

    unsafe fn acquire_texture(
        &self,
        _timeout: Option<Duration>,
        _fence: &Fence,
    ) -> Result<crate::AcquiredSurfaceTexture<Api>, crate::SurfaceError> {
        FIELDWORK_ACQUIRES.fetch_add(1, Ordering::SeqCst);
        FIELDWORK_OUTSTANDING.fetch_add(1, Ordering::SeqCst);
        Ok(crate::AcquiredSurfaceTexture {
            texture: SurfaceResource::acquired(),
            suboptimal: false,
        })
    }

    unsafe fn discard_texture(&self, mut texture: SurfaceResource) {
        texture.accounted = true;
        FIELDWORK_DISCARDS.fetch_add(1, Ordering::SeqCst);
        let previous = FIELDWORK_OUTSTANDING.fetch_sub(1, Ordering::SeqCst);
        assert!(previous > 0, "explicit surface discard underflow");
    }
""",
    "surface teardown instrumentation",
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
    "surface capabilities",
)

noop = replace_once(
    noop,
    "surface_textures: &[&Resource],",
    "surface_textures: &[&SurfaceResource],",
    "queue submit surface type",
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
        mut texture: SurfaceResource,
    ) -> Result<(), crate::SurfaceError> {
        texture.accounted = true;
        FIELDWORK_PRESENTS.fetch_add(1, Ordering::SeqCst);
        let previous = FIELDWORK_OUTSTANDING.fetch_sub(1, Ordering::SeqCst);
        assert!(previous > 0, "surface present underflow");
        Ok(())
    }
""",
    "queue present instrumentation",
)

noop_path.write_text(noop)

instance_path = Path("wgpu-core/src/instance.rs")
instance = instance_path.read_text()
instance += r'''

#[cfg(all(test, feature = "noop", target_os = "linux"))]
mod fieldwork_device_loss_teardown_tests {
    use super::*;
    use crate::{device::DeviceDescriptor, present::SurfaceError};
    use alloc::sync::Arc;
    use core::sync::atomic::Ordering;
    use raw_window_handle::{
        RawDisplayHandle, RawWindowHandle, XlibDisplayHandle, XlibWindowHandle,
    };
    use std::panic::{catch_unwind, AssertUnwindSafe};

    fn configured_noop_surface() -> (Arc<Surface>, Arc<crate::device::Device>, Arc<crate::device::queue::Queue>) {
        let mut descriptor = wgt::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgt::Backends::NOOP;
        descriptor.backend_options.noop = wgt::NoopBackendOptions::enabled();
        let instance = Instance::new("fieldwork-device-loss-teardown", descriptor, None);

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
        assert!(surface.configure(&device, &config).is_none());

        (surface, device, queue)
    }

    fn acquire_then_invalidate(
        surface: &Arc<Surface>,
        device: &Arc<crate::device::Device>,
    ) {
        let output = surface
            .get_current_texture_inner()
            .expect("controlled acquisition should return");
        assert_eq!(output.status, wgt::SurfaceStatus::Good);
        assert!(output.texture.is_some());

        device.valid.store(false, Ordering::Release);
        let error = surface
            .present_inner()
            .expect_err("invalid device must reject before queue-level detachment");
        assert!(matches!(error, SurfaceError::Device(crate::device::DeviceError::Lost)));
        assert!(
            surface
                .presentation
                .lock()
                .as_ref()
                .expect("surface should remain configured")
                .acquired_texture
                .is_some(),
            "the failed public entry must retain acquired_texture"
        );

        // Model the consumed public wrapper returning from Queue::present: its
        // texture clone is gone, but Presentation still owns the acquired frame.
        drop(output);
    }

    #[test]
    fn fieldwork_surface_drop_unconfigures_before_retained_release() {
        hal::noop::fieldwork_reset_surface_counters();
        let (surface, device, queue) = configured_noop_surface();
        acquire_then_invalidate(&surface, &device);

        let before = hal::noop::fieldwork_surface_counters();
        eprintln!("baseline before surface drop: {before:?}");
        assert_eq!(before.acquires, 1);
        assert_eq!(before.outstanding, 1);
        assert_eq!(before.implicit_releases, 0);

        let drop_result = catch_unwind(AssertUnwindSafe(|| drop(surface)));
        assert!(
            drop_result.is_err(),
            "current teardown ordering should expose the outstanding-resource precondition"
        );

        let after = hal::noop::fieldwork_surface_counters();
        eprintln!("baseline after caught teardown panic: {after:?}");
        assert_eq!(after.unconfigures, 1);
        assert_eq!(after.unconfigure_violations, 1);

        drop(queue);
        drop(device);
    }

    #[test]
    fn fieldwork_release_before_unconfigure_satisfies_noop_precondition() {
        hal::noop::fieldwork_reset_surface_counters();
        let (surface, device, queue) = configured_noop_surface();
        acquire_then_invalidate(&surface, &device);

        surface
            .release_inner()
            .expect("release_inner should not require a valid device");

        let released = hal::noop::fieldwork_surface_counters();
        eprintln!("release control before surface drop: {released:?}");
        assert_eq!(released.acquires, 1);
        assert_eq!(released.discards, 0);
        assert_eq!(released.presents, 0);
        assert_eq!(released.implicit_releases, 1);
        assert_eq!(
            released.outstanding, 0,
            "dropping the retained core texture must satisfy unconfigure ownership"
        );

        let drop_result = catch_unwind(AssertUnwindSafe(|| drop(surface)));
        assert!(
            drop_result.is_ok(),
            "release-before-unconfigure should avoid the controlled teardown panic"
        );

        let after = hal::noop::fieldwork_surface_counters();
        eprintln!("release control after surface drop: {after:?}");
        assert_eq!(after.unconfigures, 1);
        assert_eq!(after.unconfigure_violations, 0);
        assert_eq!(after.outstanding, 0);

        drop(queue);
        drop(device);
    }
}
'''
instance_path.write_text(instance)
