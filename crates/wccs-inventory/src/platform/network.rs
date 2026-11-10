use super::Buffer;
use crate::model::{IpAddress, NetworkAdapter, Observation, Reason};
use std::net::{Ipv4Addr, Ipv6Addr};
use windows::{
    Win32::{Foundation::*, NetworkManagement::IpHelper::*, Networking::WinSock::*},
    core::{PWSTR, Result as WinResult},
};

pub fn collect() -> WinResult<Vec<NetworkAdapter>> {
    let mut size = 15 * 1024;
    for _ in 0..3 {
        let mut buffer = Buffer::new(size)?;
        // SAFETY: buffer is 16-byte aligned, with at least size writable bytes.
        // AF_UNSPEC includes both address families. The API reads local configuration.
        let status = unsafe {
            GetAdaptersAddresses(
                AF_UNSPEC.0 as u32,
                GAA_FLAG_INCLUDE_GATEWAYS | GAA_FLAG_INCLUDE_ALL_INTERFACES,
                None,
                Some(buffer.as_mut_ptr().cast()),
                &mut size,
            )
        };
        if status == ERROR_BUFFER_OVERFLOW.0 {
            continue;
        }
        if status == ERROR_NO_DATA.0 {
            return Ok(Vec::new());
        }
        WIN32_ERROR(status).ok()?;
        let mut adapters = Vec::new();
        let mut current = buffer.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        // SAFETY: successful GetAdaptersAddresses initialized the linked structures
        // and embedded strings/socket addresses in buffer. Traversal completes before
        // buffer is dropped or moved, and does not mutate or retain any OS pointers.
        unsafe {
            while let Some(adapter) = current.as_ref() {
                adapters.push(read_adapter(adapter));
                current = adapter.Next;
            }
        }
        return Ok(adapters);
    }
    Err(ERROR_BUFFER_OVERFLOW.into())
}

// SAFETY contract: adapter and every embedded pointer belong to a successful
// GetAdaptersAddresses allocation that remains alive throughout this call.
unsafe fn read_adapter(adapter: &IP_ADAPTER_ADDRESSES_LH) -> NetworkAdapter {
    let mut addresses = Vec::new();
    let mut dns_servers = Vec::new();
    let mut gateways = Vec::new();
    // SAFETY: caller guarantees validity and lifetime of all linked nodes and strings.
    unsafe {
        let mut current = adapter.FirstUnicastAddress;
        while let Some(address) = current.as_ref() {
            addresses.push(IpAddress {
                address: socket_address(&address.Address),
                prefix_length: address.OnLinkPrefixLength,
            });
            current = address.Next;
        }
        let mut current = adapter.FirstDnsServerAddress;
        while let Some(address) = current.as_ref() {
            dns_servers.push(socket_address(&address.Address));
            current = address.Next;
        }
        let mut current = adapter.FirstGatewayAddress;
        while let Some(address) = current.as_ref() {
            gateways.push(socket_address(&address.Address));
            current = address.Next;
        }
        NetworkAdapter {
            name: string(adapter.FriendlyName),
            description: string(adapter.Description),
            interface_index: adapter.Anonymous1.Anonymous.IfIndex,
            ipv6_interface_index: adapter.Ipv6IfIndex,
            interface_type: adapter.IfType,
            operational_status: adapter.OperStatus.0,
            mac_address: adapter.PhysicalAddress
                [..(adapter.PhysicalAddressLength as usize).min(adapter.PhysicalAddress.len())]
                .iter()
                .map(|byte| format!("{byte:02X}"))
                .collect::<Vec<_>>()
                .join(":"),
            mtu: adapter.Mtu,
            addresses,
            dns_servers,
            gateways,
        }
    }
}

// SAFETY contract: non-null value points to an OS-owned, terminated UTF-16 string.
unsafe fn string(value: PWSTR) -> String {
    if value.is_null() {
        return String::new();
    }
    // SAFETY: caller guarantees a live terminated string. Lossy conversion preserves availability.
    unsafe { String::from_utf16_lossy(value.as_wide()) }
}

// SAFETY contract: any non-null address pointer covers iSockaddrLength readable bytes.
unsafe fn socket_address(address: &SOCKET_ADDRESS) -> Observation<String> {
    if address.lpSockaddr.is_null() || address.iSockaddrLength < size_of::<ADDRESS_FAMILY>() as i32
    {
        return Observation::unavailable(Reason::Missing);
    }
    // SAFETY: preceding checks cover the family field. Each arm checks its full
    // structure size before reading. All pointers are borrowed from the live API buffer.
    unsafe {
        let family = address.lpSockaddr.cast::<ADDRESS_FAMILY>().read_unaligned();
        match family {
            AF_INET if address.iSockaddrLength >= size_of::<SOCKADDR_IN>() as i32 => {
                let socket = address.lpSockaddr.cast::<SOCKADDR_IN>().read_unaligned();
                Observation::available(
                    Ipv4Addr::from(socket.sin_addr.S_un.S_addr.to_ne_bytes()).to_string(),
                )
            }
            AF_INET6 if address.iSockaddrLength >= size_of::<SOCKADDR_IN6>() as i32 => {
                let socket = address.lpSockaddr.cast::<SOCKADDR_IN6>().read_unaligned();
                let ip = Ipv6Addr::from(socket.sin6_addr.u.Byte);
                let scope = socket.Anonymous.sin6_scope_id;
                Observation::available(if scope == 0 {
                    ip.to_string()
                } else {
                    format!("{ip}%{scope}")
                })
            }
            _ => Observation::unavailable(Reason::Unsupported),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_socket_address_is_explicit() {
        // SAFETY: null socket pointer with zero length satisfies the helper contract.
        let result = unsafe { socket_address(&SOCKET_ADDRESS::default()) };
        assert_eq!(result, Observation::unavailable(Reason::Missing));
    }

    #[test]
    fn ipv4_network_byte_order_is_preserved() {
        let mut socket = SOCKADDR_IN {
            sin_family: AF_INET,
            ..Default::default()
        };
        socket.sin_addr.S_un.S_addr = u32::from_ne_bytes([192, 0, 2, 42]);
        let address = SOCKET_ADDRESS {
            lpSockaddr: (&mut socket as *mut SOCKADDR_IN).cast(),
            iSockaddrLength: size_of::<SOCKADDR_IN>() as i32,
        };
        // SAFETY: address points to the live local socket with its correct byte length.
        assert_eq!(
            unsafe { socket_address(&address) },
            Observation::available("192.0.2.42".into())
        );
    }
}
