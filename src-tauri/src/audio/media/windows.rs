use std::{ffi::c_void, path::Path};

use ::windows::{
    core::{factory, Result, HSTRING},
    Media::{SystemMediaTransportControls, SystemMediaTransportControlsDisplayUpdater},
    Storage::{StorageFile, Streams::RandomAccessStreamReference},
    Win32::{
        Foundation::HWND,
        System::WinRT::{
            ISystemMediaTransportControlsInterop, RoInitialize, RoUninitialize,
            RO_INIT_MULTITHREADED,
        },
    },
};

pub(super) struct Runtime;

impl Runtime {
    pub(super) fn new() -> Result<Self> {
        // The media worker owns the WinRT apartment for its whole lifetime.
        unsafe { RoInitialize(RO_INIT_MULTITHREADED)? };
        Ok(Self)
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        unsafe { RoUninitialize() };
    }
}

pub(super) struct Thumbnail(SystemMediaTransportControlsDisplayUpdater);

impl Thumbnail {
    pub(super) fn new(hwnd: u64) -> Result<Self> {
        let interop =
            factory::<SystemMediaTransportControls, ISystemMediaTransportControlsInterop>()?;
        let controls: SystemMediaTransportControls =
            unsafe { interop.GetForWindow(HWND(hwnd as usize as *mut c_void))? };
        Ok(Self(controls.DisplayUpdater()?))
    }

    pub(super) fn publish(&self, path: &Path) -> Result<()> {
        // Playwire uses CreateFromUri, which does not support local file URLs on Windows.
        let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path.as_os_str()))?.join()?;
        let stream = RandomAccessStreamReference::CreateFromFile(&file)?;
        self.0.SetThumbnail(&stream)?;
        self.0.Update()
    }
}
