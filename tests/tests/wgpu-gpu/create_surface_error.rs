//! Test that `create_surface_*()` and browser surface configuration accurately
//! report the errors we can provoke.
#![cfg(wasm_test)]

#[cfg(all(target_arch = "wasm32", not(feature = "webgl")))]
use wasm_bindgen::{JsCast, JsValue};
use wgpu_test::GpuTestInitializer;
use wgpu_test::{gpu_test, GpuTestConfiguration};

pub fn all_tests(vec: &mut Vec<GpuTestInitializer>) {
    vec.push(CANVAS_GET_CONTEXT_RETURNED_NULL);
    #[cfg(not(feature = "webgl"))]
    vec.push(REJECTED_BROWSER_CONFIGURATION_IS_PUBLISHED_AND_RECOVERABLE);
}

/// This test applies to those cfgs that can create a surface from a canvas, which
/// include WebGL and WebGPU, but *not* Emscripten GLES.
#[gpu_test]
static CANVAS_GET_CONTEXT_RETURNED_NULL: GpuTestConfiguration = GpuTestConfiguration::new()
    .parameters(wgpu_test::TestParameters::default().enable_noop())
    .run_async(|_ctx| async move {
        #[cfg(target_arch = "wasm32")]
        {
            // Not using the normal testing infrastructure because that goes straight to creating the canvas for us.
            let instance = wgpu_test::initialize_instance(
                wgpu::Backends::all(),
                &wgpu_test::TestParameters::default(),
            );
            // Create canvas
            let canvas = wgpu_test::initialize_html_canvas();

            // Using a context id that is not "webgl2" or "webgpu" will render the canvas unusable by wgpu.
            canvas.get_context("2d").unwrap();

            #[allow(
                clippy::redundant_clone,
                reason = "false positive — can't and shouldn't move out."
            )]
            let error = instance
                .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
                .unwrap_err();

            assert!(
                error
                    .to_string()
                    .contains("canvas.getContext() returned null"),
                "{error}"
            );
        }
    });

/// Characterize the public state and recovery path after the browser WebGPU
/// backend rejects a surface configuration without aborting wasm.
#[cfg(not(feature = "webgl"))]
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
                canvas.set_width(2);
                canvas.set_height(2);
                let raw_context = canvas
                    .get_context("webgpu")
                    .expect("getting the browser WebGPU context should not throw")
                    .expect("browser WebGPU context should exist");
                let surface = instance
                    .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
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
                assert!(
                    raw_context_is_configured(&raw_context),
                    "the raw browser context should expose the accepted baseline configuration"
                );

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
                // This particular color-space rejection happens before the backend calls
                // `GPUCanvasContext.configure`, so the raw canvas remains configured with
                // the earlier accepted baseline.
                surface.configure(&device, &rejected);

                // Current behavior: the public wrapper publishes the request even
                // though the browser backend did not apply it.
                assert_eq!(surface.get_configuration(), Some(rejected.clone()));
                assert!(matches!(
                    surface.get_current_texture(),
                    wgpu::CurrentSurfaceTexture::Lost
                ));

                // The underlying browser context is neither unconfigured nor lost: it
                // still exposes a configuration and can acquire a canvas texture. The
                // `Lost` result is therefore wrapper-owned failure state in this path.
                assert!(
                    raw_context_is_configured(&raw_context),
                    "rejected wgpu-only color-space mapping should not erase the browser baseline"
                );
                raw_context_acquire_and_destroy(&raw_context);

                // A supported reconfiguration on the same surface clears the wrapper's
                // failure flag and recovers without recreating either surface or device.
                surface.configure(&device, &baseline);
                assert_eq!(surface.get_configuration(), Some(baseline.clone()));
                present_success(&surface, &queue, "same-surface supported recovery");

                // Recreating a surface does not make the same unsupported
                // configuration valid. This mirrors the shared example framework's
                // current generic `Lost` recovery strategy.
                let retry_canvas = wgpu_test::initialize_html_canvas();
                retry_canvas.set_width(2);
                retry_canvas.set_height(2);
                let retry_surface = instance
                    .create_surface(wgpu::SurfaceTarget::Canvas(retry_canvas))
                    .expect("could not recreate browser WebGPU surface");
                retry_surface.configure(&device, &rejected);
                assert!(matches!(
                    retry_surface.get_current_texture(),
                    wgpu::CurrentSurfaceTexture::Lost
                ));

                // A supported fallback on the recreated surface also succeeds, proving
                // that neither the device nor the browser WebGPU implementation was lost.
                retry_surface.configure(&device, &baseline);
                assert_eq!(retry_surface.get_configuration(), Some(baseline));
                present_success(&retry_surface, &queue, "recreated-surface supported recovery");
            }
        });

#[cfg(all(target_arch = "wasm32", not(feature = "webgl")))]
fn call_raw_method(target: &JsValue, name: &str) -> Result<JsValue, JsValue> {
    let method = js_sys::Reflect::get(target, &JsValue::from_str(name))?
        .dyn_into::<js_sys::Function>()?;
    method.call0(target)
}

#[cfg(all(target_arch = "wasm32", not(feature = "webgl")))]
fn raw_context_is_configured(context: &JsValue) -> bool {
    let configuration = call_raw_method(context, "getConfiguration")
        .expect("GPUCanvasContext.getConfiguration should not throw");
    !configuration.is_null() && !configuration.is_undefined()
}

#[cfg(all(target_arch = "wasm32", not(feature = "webgl")))]
fn raw_context_acquire_and_destroy(context: &JsValue) {
    let texture = call_raw_method(context, "getCurrentTexture")
        .expect("raw GPUCanvasContext should remain able to acquire after wgpu rejection");
    call_raw_method(&texture, "destroy").expect("destroying the raw canvas texture should succeed");
}

#[cfg(all(target_arch = "wasm32", not(feature = "webgl")))]
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
