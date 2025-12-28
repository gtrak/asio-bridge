use crate::AudioRing;
use asio_sys::asio_import::{
    ASIOBufferInfo, ASIOCallbacks, ASIOCreateBuffers, ASIODriverInfo, ASIOGetBufferSize,
    ASIOGetChannels, ASIOInit, ASIOSampleRate, ASIOStart, ASIOTime, AsioDriverList, AsioDrivers,
};
use std::{ptr, sync::Arc};

static mut RING: Option<Arc<AudioRing>> = None;
static mut BUFFER_SIZE: usize = 0;
static mut CHANNELS: usize = 0;
static mut ASIO_BUFFERS: *mut ASIOBufferInfo = std::ptr::null_mut();

unsafe extern "C" fn buffer_switch(double_buffer_index: i32, direct_process: i32) {
    let ring = RING.as_ref().unwrap();

    let frames = BUFFER_SIZE;
    let chans = CHANNELS;

    let mut out = vec![0.0f32; frames * chans];

    // Access buffer data correctly
    for ch in 0..chans {
        // Access the buffer pointer for this channel
        let buffer_ptr = (*ASIO_BUFFERS.add(ch)).buffers[0] as *const f32;
        let src = std::slice::from_raw_parts(buffer_ptr, frames);
        for i in 0..frames {
            out[i * chans + ch] = src[i];
        }
    }

    ring.push(out);
}

unsafe extern "C" fn sample_rate_changed(rate: ASIOSampleRate) {}
unsafe extern "C" fn asio_message(
    selector: i32,
    value: i32,
    msg: *mut std::ffi::c_void,
    opt: *mut f64,
) -> i32 {
    0
}

unsafe extern "C" fn buffer_switch_time_info(
    params: *mut ASIOTime,
    index: i32,
    direct: i32,
) -> *mut ASIOTime {
    buffer_switch(index, direct);
    ptr::null_mut()
}

pub unsafe fn start_asio(ring: Arc<AudioRing>) -> anyhow::Result<()> {
    RING = Some(ring);

    // Create AsioDrivers instance to enumerate drivers
    let mut drivers = AsioDrivers::new();

    const MAX_DRIVERS: usize = 32;
    const MAX_NAME_LEN: usize = 32;
    let mut name_storage = vec![[0i8; MAX_NAME_LEN]; MAX_DRIVERS];
    let mut name_ptrs: Vec<*mut i8> = name_storage
        .iter_mut()
        .map(|buf| buf.as_mut_ptr())
        .collect();

    // If NUX driver not found, enumerate all drivers and find one that contains 'NUX' in its name

    let num_drivers = drivers.getDriverNames(name_ptrs.as_mut_ptr(), MAX_DRIVERS as i32);

    // Print the driver names
    for i in 0..num_drivers {
        let driver_ptr = name_ptrs[i as usize];

        if !driver_ptr.is_null() {
            let driver_cstr = std::ffi::CStr::from_ptr(driver_ptr);
            if let Ok(driver_str) = driver_cstr.to_str() {
                // Check if this is the NUX driver (case-insensitive search)
                if driver_str.contains("NUX") && drivers.loadDriver(driver_ptr) {
                    println!("Driver {}: {} Loaded", i, driver_str);
                    break;
                }
            }
        }
    }

    let mut input_channels = 0;
    let mut output_channels = 0;

    ASIOGetChannels(&mut input_channels, &mut output_channels);
    CHANNELS = input_channels as usize;

    let mut bufsize = 0;
    ASIOGetBufferSize(
        ptr::null_mut(),
        ptr::null_mut(),
        &mut bufsize,
        ptr::null_mut(),
    );
    BUFFER_SIZE = bufsize as usize;

    // Create buffer info array
    let mut buffer_infos: Vec<ASIOBufferInfo> = vec![
        ASIOBufferInfo {
            isInput: 0,
            channelNum: 0,
            buffers: [ptr::null_mut(), ptr::null_mut()],
        };
        input_channels as usize
    ];

    let mut callbacks = ASIOCallbacks {
        bufferSwitch: Some(buffer_switch),
        bufferSwitchTimeInfo: Some(buffer_switch_time_info),
        sampleRateDidChange: Some(sample_rate_changed),
        asioMessage: Some(asio_message),
    };

    ASIOCreateBuffers(
        buffer_infos.as_mut_ptr(),
        input_channels,
        bufsize,
        &mut callbacks as *mut ASIOCallbacks,
    );
    ASIO_BUFFERS = buffer_infos.as_mut_ptr();
    ASIOStart();

    Ok(())
}
