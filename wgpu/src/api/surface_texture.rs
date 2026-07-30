use crate::*;

/// Surface texture that can be rendered to.
/// Result of a successful call to [`Surface::get_current_texture`].
///
/// This type is unique to the Rust API of `wgpu`. In the WebGPU specification,
/// the [`GPUCanvasContext`](https://gpuweb.github.io/gpuweb/#canvas-context) provides
/// a texture without any additional information.
#[derive(Debug, Clone)]
pub struct SurfaceTexture {
    /// Accessible view of the frame.
    pub texture: Texture,
    pub(crate) presented: bool,
    pub(crate) detail: dispatch::DispatchSurfaceOutputDetail,
}
#[cfg(send_sync)]
static_assertions::assert_impl_all!(SurfaceTexture: Send, Sync);

crate::cmp::impl_eq_ord_hash_proxy!(SurfaceTexture => .texture.inner);

impl SurfaceTexture {
    #[cfg(custom)]
    /// Returns custom implementation of SurfaceTexture (if custom backend and is internally T)
    pub fn as_custom<T: crate::custom::SurfaceOutputDetailInterface>(&self) -> Option<&T> {
        self.detail.as_custom()
    }
}

impl Drop for SurfaceTexture {
    fn drop(&mut self) {
        if !self.presented {
            if thread_panicking() {
                // Best effort: release reference to `SwapchainAcquireSemaphore`
                // This fixes <https://github.com/gfx-rs/wgpu/issues/8243>
                // `Trying to destroy a SwapchainAcquireSemaphore that is still in use by a SurfaceTexture`
                self.detail.texture_release();
            } else {
                self.detail.texture_discard();
            }
        }
    }
}

/// Result of a call to [`Surface::get_current_texture`].
///
/// See variant documentation for how to handle each case.
#[derive(Debug)]
pub enum CurrentSurfaceTexture {
    /// Successfully acquired a surface texture with no issues.
    Success(SurfaceTexture),
    /// Successfully acquired a surface texture, but texture no longer matches the properties of the underlying surface.
    /// It's highly recommended to call [`Surface::configure`] again for optimal performance.
    Suboptimal(SurfaceTexture),
    /// A timeout was encountered while trying to acquire the next frame.
    ///
    /// Applications should skip the current frame and try again later.
    Timeout,
    /// The window is occluded (e.g. minimized or behind another window).
    ///
    /// Applications should skip the current frame and try again once the window
    /// is no longer occluded.
    Occluded,
    /// The underlying surface has changed, and therefore the surface configuration is outdated.
    ///
    /// Call [`Surface::configure()`] and try again.
    Outdated,
    /// The surface has been lost and needs to be recreated.
    ///
    /// If the device as a whole is lost (see [`set_device_lost_callback()`][crate::Device::set_device_lost_callback]), then
    /// you need to recreate the device and all resources.
    /// Otherwise, call [`Instance::create_surface()`] to recreate the surface,
    /// then [`Surface::configure()`], and try again.
    Lost,
    /// A validation error inside [`Surface::get_current_texture()`] was raised
    /// and caught by an [error scope](crate::Device::push_error_scope) or
    /// [`on_uncaptured_error()`][crate::Device::on_uncaptured_error].
    ///
    /// Applications should attend to the validation error and try again.
    Validation,
}

fn thread_panicking() -> bool {
    cfg_if::cfg_if! {
        if #[cfg(std)] {
            std::thread::panicking()
        } else if #[cfg(panic = "abort")] {
            // If `panic = "abort"` then a thread _cannot_ be observably panicking by definition.
            false
        } else {
            // TODO: This is potentially overly pessimistic; it may be appropriate to instead allow a
            // texture to not be discarded.
            // Alternatively, this could _also_ be a `panic!`, since we only care if the thread is panicking
            // when the surface has not been presented.
            compile_error!(
                "cannot determine if a thread is panicking without either `panic = \"abort\"` or `std`"
            );
        }
    }
}

#[cfg(all(test, custom, std))]
mod tests {
    use super::*;
    use std::{
        panic::{catch_unwind, AssertUnwindSafe},
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
    };

    #[derive(Debug)]
    struct TestTexture;

    impl custom::TextureInterface for TestTexture {
        fn create_view(
            &self,
            _desc: &TextureViewDescriptor<'_>,
        ) -> custom::DispatchTextureView {
            unimplemented!("the ownership characterization never creates a texture view")
        }

        fn destroy(&self) {}
    }

    #[derive(Debug)]
    struct RecordingOutputDetail {
        discard_calls: Arc<AtomicUsize>,
        release_calls: Arc<AtomicUsize>,
    }

    impl custom::SurfaceOutputDetailInterface for RecordingOutputDetail {
        fn texture_discard(&self) {
            self.discard_calls.fetch_add(1, Ordering::SeqCst);
        }

        fn texture_release(&self) {
            self.release_calls.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[derive(Debug)]
    struct PanickingPresentQueue {
        present_calls: Arc<AtomicUsize>,
    }

    impl custom::QueueInterface for PanickingPresentQueue {
        fn write_buffer(
            &self,
            _buffer: &custom::DispatchBuffer,
            _offset: BufferAddress,
            _data: &[u8],
        ) {
            unimplemented!()
        }

        fn create_staging_buffer(
            &self,
            _size: BufferSize,
        ) -> Option<custom::DispatchQueueWriteBuffer> {
            unimplemented!()
        }

        fn validate_write_buffer(
            &self,
            _buffer: &custom::DispatchBuffer,
            _offset: BufferAddress,
            _size: BufferSize,
        ) -> Option<()> {
            unimplemented!()
        }

        fn write_staging_buffer(
            &self,
            _buffer: &custom::DispatchBuffer,
            _offset: BufferAddress,
            _staging_buffer: &custom::DispatchQueueWriteBuffer,
        ) {
            unimplemented!()
        }

        fn write_texture(
            &self,
            _texture: TexelCopyTextureInfo<'_>,
            _data: &[u8],
            _data_layout: TexelCopyBufferLayout,
            _size: Extent3d,
        ) {
            unimplemented!()
        }

        #[cfg(all(target_arch = "wasm32", feature = "web"))]
        fn copy_external_image_to_texture(
            &self,
            _source: &CopyExternalImageSourceInfo,
            _dest: CopyExternalImageDestInfo<&Texture>,
            _size: Extent3d,
        ) {
            unimplemented!()
        }

        fn submit(
            &self,
            _command_buffers: &mut dyn Iterator<Item = custom::DispatchCommandBuffer>,
        ) -> u64 {
            unimplemented!()
        }

        fn get_timestamp_period(&self) -> f32 {
            unimplemented!()
        }

        fn on_submitted_work_done(&self, _callback: custom::BoxSubmittedWorkDoneCallback) {
            unimplemented!()
        }

        fn compact_blas(
            &self,
            _blas: &custom::DispatchBlas,
        ) -> (Option<u64>, custom::DispatchBlas) {
            unimplemented!()
        }

        fn present(&self, _detail: &custom::DispatchSurfaceOutputDetail) {
            self.present_calls.fetch_add(1, Ordering::SeqCst);
            panic!("injected present failure after public ownership commit");
        }
    }

    fn test_surface_texture(
        discard_calls: Arc<AtomicUsize>,
        release_calls: Arc<AtomicUsize>,
    ) -> SurfaceTexture {
        let descriptor = TextureDescriptor {
            label: Some("surface presentation ownership characterization"),
            size: Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        };

        SurfaceTexture {
            texture: Texture::from_custom(TestTexture, &descriptor),
            presented: false,
            detail: custom::DispatchSurfaceOutputDetail::custom(RecordingOutputDetail {
                discard_calls,
                release_calls,
            }),
        }
    }

    #[test]
    fn ordinary_unpresented_drop_discards() {
        let discard_calls = Arc::new(AtomicUsize::new(0));
        let release_calls = Arc::new(AtomicUsize::new(0));

        drop(test_surface_texture(
            Arc::clone(&discard_calls),
            Arc::clone(&release_calls),
        ));

        assert_eq!(discard_calls.load(Ordering::SeqCst), 1);
        assert_eq!(release_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn present_panic_does_not_discard_or_release_after_ownership_commit() {
        let discard_calls = Arc::new(AtomicUsize::new(0));
        let release_calls = Arc::new(AtomicUsize::new(0));
        let present_calls = Arc::new(AtomicUsize::new(0));
        let queue = Queue::from_custom(PanickingPresentQueue {
            present_calls: Arc::clone(&present_calls),
        });
        let surface_texture = test_surface_texture(
            Arc::clone(&discard_calls),
            Arc::clone(&release_calls),
        );

        let result = catch_unwind(AssertUnwindSafe(|| queue.present(surface_texture)));

        assert!(result.is_err());
        assert_eq!(present_calls.load(Ordering::SeqCst), 1);
        assert_eq!(discard_calls.load(Ordering::SeqCst), 0);
        assert_eq!(release_calls.load(Ordering::SeqCst), 0);
    }
}
