//! Characterize browser surface state after a rejected configuration.
#![cfg(wasm_test)]

use wgpu_test::{gpu_test, GpuTestConfiguration, GpuTestInitializer};

pub fn all_tests(tests: &mut Vec<GpuTestInitializer>) {
    tests.push(REJECTED_BROWSER_CONFIGURATION_IS_PUBLISHED_AND_RECOVERABLE);
}

#[gpu_test]
static REJECTED_BROWSER_CONFIGURATION_IS_PUBLISHED_AND_RECOVERABLE: GpuTestConfiguration =
    GpuTestConfiguration::new()
        .parameters(wgpu_test::TestParameters::default().enable_noop())
        .run_async(|_ctx| async move {
            #[cfg(target_arch = "wasm32")]
            {
                let instance = wgpu_test::initialize_instance(
                    wgpu::Backends::BROWSER_WEBGPU,
                    &wgpu_test::TestParameters::default(),
                );
                let canvas = wgpu_test::initialize_html_canvas();
                let surface = instance
                    .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
                    .expect("could not create browser WebGPU surface");

                let adapter = instance
                    .request_adapter(&wgpu::RequestAdapterOptions {
                        compatible_surface: Some(&surface),
                        ..Default::default()
                    })
                    .await
                    .expect("could not find a browser WebGPU adapter");
                let (device, queue) = adapter
                    .request_device(&wgpu::DeviceDescriptor::default())
                    .await
                    .expect("could not create browser WebGPU device");

                let baseline = surface
                    .get_default_config(&adapter, 2, 2)
                    .expect("surface should have a supported default configuration");
                surface.configure(&device, &baseline);
                present_success(&surface, &queue, "baseline configuration");
                assert_eq!(surface.get_configuration(), Some(baseline.clone()));

                let mut rejected = baseline.clone();
                rejected.color_space = wgpu::SurfaceColorSpace::ExtendedSrgbLinear;
                assert!(
                    !surface
                        .get_capabilities(&adapter)
                        .color_spaces(rejected.format)
                        .contains(wgpu::SurfaceColorSpaces::EXTENDED_SRGB_LINEAR),
                    "the test configuration must be unsupported by browser WebGPU"
                );

                // Browser WebGPU deliberately contains this rejection instead of
                // allowing a JavaScript exception to become an unrecoverable wasm abort.
                surface.configure(&device, &rejected);

                // Current behavior: the public wrapper publishes the request even
                // though the browser backend did not apply it.
                assert_eq!(surface.get_configuration(), Some(rejected.clone()));
                assert!(matches!(
                    surface.get_current_texture(),
                    wgpu::CurrentSurfaceTexture::Lost
                ));

                // Recreating a surface does not make the same unsupported
                // configuration valid. This mirrors the shared example framework's
                // current generic `Lost` recovery strategy.
                let retry_canvas = wgpu_test::initialize_html_canvas();
                let retry_surface = instance
                    .create_surface(wgpu::SurfaceTarget::Canvas(retry_canvas))
                    .expect("could not recreate browser WebGPU surface");
                retry_surface.configure(&device, &rejected);
                assert!(matches!(
                    retry_surface.get_current_texture(),
                    wgpu::CurrentSurfaceTexture::Lost
                ));

                // A supported fallback clears the backend failure state and proves
                // that neither the device nor the browser WebGPU implementation was lost.
                retry_surface.configure(&device, &baseline);
                assert_eq!(retry_surface.get_configuration(), Some(baseline));
                present_success(&retry_surface, &queue, "supported fallback configuration");
            }
        });

#[cfg(target_arch = "wasm32")]
fn present_success(surface: &wgpu::Surface<'_>, queue: &wgpu::Queue, phase: &str) {
    let frame = match surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(frame)
        | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
        wgpu::CurrentSurfaceTexture::Timeout => panic!("{phase}: timed out acquiring a frame"),
        wgpu::CurrentSurfaceTexture::Occluded => panic!("{phase}: surface was occluded"),
        wgpu::CurrentSurfaceTexture::Outdated => panic!("{phase}: surface was outdated"),
        wgpu::CurrentSurfaceTexture::Lost => panic!("{phase}: surface was lost"),
        wgpu::CurrentSurfaceTexture::Validation => {
            panic!("{phase}: acquisition produced a validation error")
        }
    };
    queue.present(frame);
}
