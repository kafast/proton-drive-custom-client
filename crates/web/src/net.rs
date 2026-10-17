//! Networks allowed to access the web interface (e.g. `192.168.0.0/16`).

use std::fmt;
use std::net::IpAddr;
use std::str::FromStr;

/// An IP network in CIDR notation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpNetwork {
    address: IpAddr,
    prefix: u8,
}

/// Error when parsing a network.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid network {0:?} (example: 192.168.0.0/16)")]
pub struct InvalidNetwork(pub String);

/// Default: only the local machine and private networks (home network, VPN, containers).
pub const PRIVATE_NETWORKS: [&str; 7] = [
    "127.0.0.0/8",
    "10.0.0.0/8",
    "172.16.0.0/12",
    "192.168.0.0/16",
    "169.254.0.0/16",
    "::1/128",
    "fc00::/7",
];

impl IpNetwork {
    /// `true` if `ip` lies in this network. IPv4 addresses in IPv6 form
    /// (`::ffff:192.168.1.2`) count as the IPv4 address.
    #[must_use]
    pub fn contains(&self, ip: IpAddr) -> bool {
        let ip = match ip {
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(IpAddr::V6(v6), IpAddr::V4),
            v4 @ IpAddr::V4(_) => v4,
        };
        match (self.address, ip) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => prefix_matches(
                u128::from(u32::from(net)),
                u128::from(u32::from(ip)),
                32,
                self.prefix,
            ),
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                prefix_matches(u128::from(net), u128::from(ip), 128, self.prefix)
            }
            _ => false,
        }
    }

    /// The default private networks.
    #[must_use]
    pub fn private() -> Vec<Self> {
        PRIVATE_NETWORKS
            .iter()
            .filter_map(|text| text.parse().ok())
            .collect()
    }
}

fn prefix_matches(network: u128, ip: u128, bits: u8, prefix: u8) -> bool {
    let shift = u32::from(bits - prefix);
    // With prefix 0 the shift would equal the bit width; then everything matches.
    shift >= u32::from(bits) || (network >> shift) == (ip >> shift)
}

impl FromStr for IpNetwork {
    type Err = InvalidNetwork;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let invalid = || InvalidNetwork(text.to_owned());
        let (address, prefix) = match text.split_once('/') {
            Some((address, prefix)) => (address, Some(prefix)),
            None => (text, None),
        };
        let address: IpAddr = address.trim().parse().map_err(|_| invalid())?;
        let bits = if address.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            Some(prefix) => prefix.trim().parse::<u8>().map_err(|_| invalid())?,
            None => bits,
        };
        if prefix > bits {
            return Err(invalid());
        }
        Ok(Self { address, prefix })
    }
}

impl fmt::Display for IpNetwork {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.address, self.prefix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net(text: &str) -> IpNetwork {
        text.parse().unwrap()
    }

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn matches_ipv4_and_ipv6_networks() {
        assert!(net("192.168.0.0/16").contains(ip("192.168.178.20")));
        assert!(!net("192.168.0.0/16").contains(ip("192.169.0.1")));
        assert!(net("10.1.2.3").contains(ip("10.1.2.3")));
        assert!(!net("10.1.2.3").contains(ip("10.1.2.4")));
        assert!(net("0.0.0.0/0").contains(ip("8.8.8.8")));
        assert!(net("fc00::/7").contains(ip("fd12::1")));
        assert!(!net("fc00::/7").contains(ip("2001:db8::1")));
        assert!(net("::/0").contains(ip("2001:db8::1")));
        assert!(!net("10.0.0.0/8").contains(ip("::1")));
    }

    #[test]
    fn mapped_ipv4_counts_as_ipv4() {
        assert!(net("192.168.0.0/16").contains(ip("::ffff:192.168.1.2")));
    }

    #[test]
    fn private_defaults_exclude_the_internet() {
        let private = IpNetwork::private();
        assert_eq!(private.len(), PRIVATE_NETWORKS.len());
        let allowed = |text| private.iter().any(|n| n.contains(ip(text)));
        assert!(allowed("127.0.0.1") && allowed("192.168.1.5") && allowed("::1"));
        assert!(!allowed("8.8.8.8") && !allowed("2001:db8::1"));
    }

    #[test]
    fn rejects_invalid_networks() {
        for bad in ["", "1.2.3", "1.2.3.4/33", "::/129", "a/8", "1.2.3.4/x"] {
            assert!(bad.parse::<IpNetwork>().is_err(), "{bad:?}");
        }
        assert_eq!(net("10.0.0.0/8").to_string(), "10.0.0.0/8");
    }
}
