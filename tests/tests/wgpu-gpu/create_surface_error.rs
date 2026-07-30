//! Test that `create_surface_*()` and browser surface configuration accurately
//! report the errors we can provoke.
#![cfg(wasm_test)]

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::{JsCast, JsValue};
use wgpu_test::GpuTestInitializer;
use wgpu_test::{gpu_test, GpuTestConfiguration};

pub fn all_tests(vec: &mut Vec<GpuTestInitializer>) {
    vec.push(CANVAS_GET_CONTEXT_RETURNED_NULL);
    vec.push(UNCONFIGURED_BROWSER_SURFACE_REPORTS_LOST);
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

/// Characterize the browser-WebGPU result for acquisition before any successful
/// configuration. Raw WebGPU reports an invalid-state exception in this state,
/// while the current wgpu browser backend contains that exception as `Lost`.
#[gpu_test]
static UNCONFIGURED_BROWSER_SURFACE_REPORTS_LOST: GpuTestConfiguration =
    GpuTestConfiguration::new()
        .parameters(wgpu_test::TestParameters::default().enable_noop())
        .run_async(|_ctx| async move {
            #[cfg(target_arch = "wasm32")]
            {
                let instance = browser_webgpu_instance();
                let canvas = wgpu_test::initialize_html_canvas();
                let raw_context = canvas
                    .get_context("webgpu")
                    .expect("getting the browser WebGPU context should not throw")
                    .expect("browser WebGPU context should exist");
                let surface = instance
                    .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
                    .expect("could not create browser WebGPU surface");

                assert_eq!(surface.get_configuration(), None);
                assert!(
                    !raw_context_is_configured(&raw_context),
                    "raw browser context should begin unconfigured"
                );
                assert!(matches!(
                    surface.get_current_texture(),
                    wgpu::CurrentSurfaceTexture::Lost
                ));
                assert!(
                    !raw_context_is_configured(&raw_context),
                    "failed acquisition should not configure the raw browser context"
                );
            }
        });

/// Characterize the public state and recovery path after the browser WebGPU
/// backend rejects a surface configuration without aborting wasm.
#[gpu_test]
static REJECTED_BROWSER_CONFIGURATION_IS_PUBLISHED_AND_RECOVERABLE: GpuTestConfiguration =
    GpuTestConfiguration::new()
        .parameters(wgpu_test::TestParameters::default().enable_noop())
        .run_async(|_ctx| async move {
            #[cfg(target_arch = "wasm32")]
            {
                let instance = browser_webgpu_instance();
                let canvas = wgpu_test::initialize_html_canvas();
                canvas.set_width(2);
                canvas.set_height(2);
                let observed_canvas = canvas.clone();
                let raw_context = canvas
                    .get_context("webgpu")
                    .expect("getting the browser WebGPU context should not throw")
                    .expect("browser WebGPU context should exist");
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
                assert_eq!(
                    adapter.get_info().backend,
                    wgpu::Backend::BrowserWebGpu,
                    "the characterization must not silently fall back to WebGL"
                );
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
                assert_eq!((observed_canvas.width(), observed_canvas.height()), (2, 2));

                let mut rejected = baseline.clone();
                rejected.width = 7;
                rejected.height = 5;
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
                // The backend updates the canvas extent first, then rejects this color
                // space before calling `GPUCanvasContext.configure`. Raw configuration
                // therefore remains the earlier accepted baseline while the canvas and
                // public wgpu cache already expose fields from the rejected request.
                surface.configure(&device, &rejected);

                assert_eq!(surface.get_configuration(), Some(rejected.clone()));
                assert_eq!(
                    (observed_canvas.width(), observed_canvas.height()),
                    (rejected.width, rejected.height),
                    "canvas extent is mutated before browser configuration rejection"
                );
                assert!(matches!(
                    surface.get_current_texture(),
                    wgpu::CurrentSurfaceTexture::Lost
                ));

                // The underlying browser context is neither unconfigured nor lost: it
                // still exposes its accepted configuration and can acquire a texture.
                // The acquired raw texture follows the resized canvas extent, showing a
                // mixed state rather than simple preservation of the complete baseline.
                assert!(
                    raw_context_is_configured(&raw_context),
                    "rejected wgpu-only color-space mapping should not erase the browser baseline"
                );
                raw_context_acquire_assert_size_and_destroy(
                    &raw_context,
                    rejected.width,
                    rejected.height,
                );

                // A supported reconfiguration on the same surface clears the wrapper's
                // failure flag and recovers without recreating either surface or device.
                surface.configure(&device, &baseline);
                assert_eq!(surface.get_configuration(), Some(baseline.clone()));
                assert_eq!((observed_canvas.width(), observed_canvas.height()), (2, 2));
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

#[cfg(target_arch = "wasm32")]
#[derive(Debug)]
struct BrowserWebGpuDisplayHandle;

#[cfg(target_arch = "wasm32")]
impl raw_window_handle::HasDisplayHandle for BrowserWebGpuDisplayHandle {
    fn display_handle(
        &self,
    ) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        Ok(raw_window_handle::DisplayHandle::web())
    }
}

#[cfg(target_arch = "wasm32")]
fn browser_webgpu_instance() -> wgpu::Instance {
    let mut descriptor =
        wgpu::InstanceDescriptor::new_with_display_handle(Box::new(BrowserWebGpuDisplayHandle));
    descriptor.backends = wgpu::Backends::BROWSER_WEBGPU;
    descriptor.flags = wgpu::InstanceFlags::debugging();
    wgpu::Instance::new(descriptor)
}

#[cfg(target_arch = "wasm32")]
fn call_raw_method(target: &JsValue, name: &str) -> Result<JsValue, JsValue> {
    let method = js_sys::Reflect::get(target, &JsValue::from_str(name))?
        .dyn_into::<js_sys::Function>()?;
    method.call0(target)
}

#[cfg(target_arch = "wasm32")]
fn raw_context_is_configured(context: &JsValue) -> bool {
    let configuration = call_raw_method(context, "getConfiguration")
        .expect("GPUCanvasContext.getConfiguration should not throw");
    !configuration.is_null() && !configuration.is_undefined()
}

#[cfg(target_arch = "wasm32")]
fn raw_u32_property(target: &JsValue, name: &str) -> u32 {
    js_sys::Reflect::get(target, &JsValue::from_str(name))
        .unwrap_or_else(|error| panic!("reading raw {name} property failed: {error:?}"))
        .as_f64()
        .unwrap_or_else(|| panic!("raw {name} property was not numeric")) as u32
}

#[cfg(target_arch = "wasm32")]
fn raw_context_acquire_assert_size_and_destroy(
    context: &JsValue,
    expected_width: u32,
    expected_height: u32,
) {
    let texture = call_raw_method(context, "getCurrentTexture")
        .expect("raw GPUCanvasContext should remain able to acquire after wgpu rejection");
    assert_eq!(raw_u32_property(&texture, "width"), expected_width);
    assert_eq!(raw_u32_property(&texture, "height"), expected_height);
    call_raw_method(&texture, "destroy").expect("destroying the raw canvas texture should succeed");
}

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
