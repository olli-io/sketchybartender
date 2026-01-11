use mach2::bootstrap::*;
use mach2::kern_return::*;
use mach2::mach_port::*;
use mach2::message::*;
use mach2::port::*;
use mach2::task::*;
use mach2::task_special_ports::*;
use mach2::traps::*;
use std::ffi::CString;
use std::os::raw::c_void;
use std::sync::Mutex;

const SKETCHYBAR_MACH_SERVICE: &str = "git.felix.sketchybar";

/// Out-of-line descriptor for 64-bit systems
/// This matches the mach_msg_ool_descriptor64_t from the system headers
/// On 64-bit macOS, the layout is:
///   - address (8 bytes)
///   - bit-fields: deallocate, copy, pad1, type (4 bytes total)
///   - size (4 bytes)
#[repr(C)]
struct MachMsgOolDescriptor64 {
    address: *mut c_void,  // 8 bytes
    /// Bit-fields packed into 4 bytes
    /// deallocate: 8 bits, copy: 8 bits, pad1: 8 bits, type: 8 bits
    bitfields: u32,
    size: u32,             // 4 bytes (comes AFTER bitfields on 64-bit!)
}

impl MachMsgOolDescriptor64 {
    fn new(address: *mut c_void, size: u32, deallocate: u8, copy: u8, type_: u8) -> Self {
        // Pack bit-fields: [deallocate:8][copy:8][pad1:8][type:8]
        let bitfields = (deallocate as u32)
            | ((copy as u32) << 8)
            | ((type_ as u32) << 24);

        Self {
            address,
            bitfields,
            size,
        }
    }
}

/// Mach message structure matching the C implementation exactly
/// Note: The C implementation uses msgh_descriptor_count directly, not wrapped in mach_msg_body_t
/// packed(4) prevents Rust from adding padding before the descriptor
#[repr(C, packed(4))]
struct MachMessage {
    header: mach_msg_header_t,
    msgh_descriptor_count: u32,  // This is mach_msg_size_t, which is u32
    descriptor: MachMsgOolDescriptor64,
}

/// Mach buffer for receiving responses
#[repr(C)]
struct MachBuffer {
    message: MachMessage,
    trailer: mach_msg_trailer_t,
}

/// Global mach port cache (lazy initialization)
static MACH_PORT: Mutex<Option<mach_port_t>> = Mutex::new(None);

/// Get the bootstrap port for sketchybar
fn get_sketchybar_port() -> Result<mach_port_t, String> {
    unsafe {
        let task = mach_task_self();

        let mut bs_port: mach_port_t = 0;
        let kr = task_get_special_port(task, TASK_BOOTSTRAP_PORT, &mut bs_port);
        if kr != KERN_SUCCESS {
            return Err(format!("Failed to get bootstrap port: {}", kr));
        }

        let service_name = CString::new(SKETCHYBAR_MACH_SERVICE)
            .map_err(|e| format!("Invalid service name: {}", e))?;

        let mut port: mach_port_t = 0;
        let kr = bootstrap_look_up(bs_port, service_name.as_ptr(), &mut port);
        if kr != KERN_SUCCESS {
            return Err(format!("Failed to lookup sketchybar service: {}", kr));
        }

        Ok(port)
    }
}

/// Format a command message for sketchybar (converting spaces outside quotes to null bytes)
/// This matches the C implementation's formatting logic
fn format_message(message: &str) -> Vec<u8> {
    let message_bytes = message.as_bytes();
    let mut formatted = Vec::with_capacity(message.len() + 1);
    let mut quote = 0u8; // 0 means no quote, otherwise it's the quote character

    for &byte in message_bytes {
        // Handle quote characters
        if byte == b'"' || byte == b'\'' {
            if quote == byte {
                // Closing quote
                quote = 0;
            } else if quote == 0 {
                // Opening quote
                quote = byte;
            } else {
                // Inside a different quote type, keep it
                formatted.push(byte);
            }
            // Don't push the quote character itself
            continue;
        }

        // Convert spaces to null bytes if not inside quotes
        if byte == b' ' && quote == 0 {
            formatted.push(0);
        } else {
            formatted.push(byte);
        }
    }

    // Remove trailing null if present and previous byte is also null
    if formatted.len() > 1
        && formatted[formatted.len() - 1] == 0
        && formatted[formatted.len() - 2] == 0
    {
        formatted.pop();
    }

    formatted.push(0); // Final null terminator
    formatted
}

/// Send a message to sketchybar via mach port and optionally receive a response
fn send_message(port: mach_port_t, message: &[u8]) -> Result<Option<String>, String> {
    unsafe {
        let task = mach_task_self();

        // Allocate a response port
        let mut response_port: mach_port_t = 0;
        let kr = mach_port_allocate(task, MACH_PORT_RIGHT_RECEIVE, &mut response_port);
        if kr != KERN_SUCCESS {
            return Err(format!("Failed to allocate response port: {}", kr));
        }

        // Insert send right
        let kr = mach_port_insert_right(
            task,
            response_port,
            response_port,
            MACH_MSG_TYPE_MAKE_SEND,
        );
        if kr != KERN_SUCCESS {
            mach_port_mod_refs(task, response_port, MACH_PORT_RIGHT_RECEIVE, -1);
            return Err(format!("Failed to insert right: {}", kr));
        }

        // Prepare the message - matching C implementation exactly
        let mut msg = MachMessage {
            header: mach_msg_header_t {
                msgh_bits: MACH_MSGH_BITS(MACH_MSG_TYPE_COPY_SEND, MACH_MSG_TYPE_MAKE_SEND)
                    | MACH_MSGH_BITS_COMPLEX,
                msgh_size: std::mem::size_of::<MachMessage>() as u32,
                msgh_remote_port: port,
                msgh_local_port: response_port,
                msgh_voucher_port: MACH_PORT_NULL,
                msgh_id: response_port as i32,
            },
            msgh_descriptor_count: 1,
            descriptor: MachMsgOolDescriptor64::new(
                message.as_ptr() as *mut _,
                message.len() as u32,
                0, // deallocate
                MACH_MSG_VIRTUAL_COPY as u8, // copy
                MACH_MSG_OOL_DESCRIPTOR as u8, // type
            ),
        };

        // Send the message
        let kr = mach_msg(
            &mut msg.header as *mut _,
            MACH_SEND_MSG,
            msg.header.msgh_size,
            0,
            MACH_PORT_NULL,
            MACH_MSG_TIMEOUT_NONE,
            MACH_PORT_NULL,
        );

        if kr != KERN_SUCCESS {
            mach_port_mod_refs(task, response_port, MACH_PORT_RIGHT_RECEIVE, -1);
            mach_port_deallocate(task, response_port);
            return Err(format!("Failed to send message: {} (0x{:x})", kr, kr));
        }

        // Receive the response with timeout
        let mut buffer: MachBuffer = unsafe { std::mem::zeroed() };

        let kr = mach_msg(
            &mut buffer.message.header as *mut _,
            MACH_RCV_MSG | MACH_RCV_TIMEOUT,
            0,
            std::mem::size_of::<MachBuffer>() as u32,
            response_port,
            1000, // 1 second timeout
            MACH_PORT_NULL,
        );

        // Clean up the response port
        mach_port_mod_refs(task, response_port, MACH_PORT_RIGHT_RECEIVE, -1);
        mach_port_deallocate(task, response_port);

        if kr != KERN_SUCCESS {
            if kr == MACH_RCV_TIMED_OUT as i32 {
                return Ok(None); // Timeout is okay, sketchybar might not respond
            }
            return Err(format!("Failed to receive response: {}", kr));
        }

        // Extract the response if available
        if !buffer.message.descriptor.address.is_null() {
            let response_ptr = buffer.message.descriptor.address as *const u8;
            let response_len = buffer.message.descriptor.size as usize;

            if response_len > 0 {
                let response_bytes = std::slice::from_raw_parts(response_ptr, response_len);
                if let Some(null_pos) = response_bytes.iter().position(|&b| b == 0) {
                    if let Ok(response) = String::from_utf8(response_bytes[..null_pos].to_vec()) {
                        // Destroy the message to deallocate OOL memory
                        mach_msg_destroy(&mut buffer.message.header as *mut _);
                        return Ok(Some(response));
                    }
                }
            }

            // Destroy the message to deallocate OOL memory
            mach_msg_destroy(&mut buffer.message.header as *mut _);
        }

        Ok(None)
    }
}

/// Send a command to sketchybar
pub fn sketchybar(command: &str) -> Result<Option<String>, String> {
    // Get or initialize the mach port
    let port = {
        let mut cached_port = MACH_PORT.lock().unwrap();
        if let Some(port) = *cached_port {
            port
        } else {
            let port = get_sketchybar_port()?;
            *cached_port = Some(port);
            port
        }
    };

    // Format the message
    let formatted = format_message(command);

    // Send the message
    send_message(port, &formatted)
}

/// Reset the cached mach port (useful if sketchybar restarts)
pub fn reset_port() {
    let mut cached_port = MACH_PORT.lock().unwrap();
    *cached_port = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_message() {
        let msg = format_message("--set item label=\"hello world\"");
        // Should convert to: --set\0item\0label=hello world\0
        assert_eq!(msg, b"--set\0item\0label=hello world\0");
    }

    #[test]
    fn test_format_message_single_quotes() {
        let msg = format_message("--set item label='hello world'");
        assert_eq!(msg, b"--set\0item\0label=hello world\0");
    }

    #[test]
    fn test_format_message_no_quotes() {
        let msg = format_message("--set item icon=X");
        assert_eq!(msg, b"--set\0item\0icon=X\0");
    }
}
