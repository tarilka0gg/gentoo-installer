//! Every graphics adapter in the machine, and what to do about each of them.
//!
//! The older detection picked a single GPU (the "best" vendor) and wrote only its driver into
//! `VIDEO_CARDS`. On a hybrid laptop that leaves Mesa without a driver for the integrated GPU the
//! screen is usually wired to, and the values themselves were wrong for Gentoo (`i915`, a bare
//! `amdgpu`, `xe`): modern Intel needs `intel` (the `iris` driver), AMD needs `amdgpu radeonsi`.
//! Here the whole PCI bus is read, so that
//!
//! * `VIDEO_CARDS` covers *every* adapter ([`video_cards`]),
//! * the compositor's render device is chosen deliberately, discrete GPU first ([`pick_render`]),
//! * virtual GPUs (virtio, VMware, QXL) get their Mesa driver too, which is what lets a VM show a desktop.

use crate::hardware::Gpu;

/// Who made an adapter, as far as driver selection cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vendor {
    /// Integrated Intel graphics (i915/xe kernel driver, Mesa `iris`).
    Intel,
    /// Intel Arc and newer discrete cards (device ids `56xx`, `e2xx`): same Mesa driver, but discrete.
    IntelArc,
    Amd,
    Nvidia,
    Virtio,
    Vmware,
    Qxl,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuDevice {
    pub vendor: Vendor,
    /// `0000:01:00.0` — the PCI address with its domain, as `lspci -D` prints it.
    pub pci_addr: String,
    /// The 4-hex-digit PCI device id.
    pub device_id: String,
    pub description: String,
}

/// Which GPU the compositor should render on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderPreference {
    /// The most capable adapter: discrete first. The default.
    #[default]
    Auto,
    /// The integrated GPU (saves power on a hybrid laptop; the screen is usually wired to it).
    Integrated,
}

impl Vendor {
    /// Higher is more capable; virtual adapters rank lowest so a real one always wins.
    fn rank(self) -> u8 {
        match self {
            Vendor::Nvidia | Vendor::IntelArc => 5,
            Vendor::Amd => 3,
            Vendor::Intel => 2,
            Vendor::Virtio | Vendor::Vmware | Vendor::Qxl | Vendor::Other => 0,
        }
    }

    fn is_virtual(self) -> bool {
        matches!(self, Vendor::Virtio | Vendor::Vmware | Vendor::Qxl)
    }
}

impl GpuDevice {
    /// Discrete adapters: NVIDIA and Intel Arc always; AMD only when an Intel GPU is there as well (an AMD
    /// APU next to an NVIDIA card is the integrated one, and a lone AMD adapter is the only choice anyway).
    pub fn is_discrete(&self, all: &[GpuDevice]) -> bool {
        match self.vendor {
            Vendor::Nvidia | Vendor::IntelArc => true,
            Vendor::Amd => all.iter().any(|d| d.vendor == Vendor::Intel) && !all.iter().any(|d| matches!(d.vendor, Vendor::Nvidia | Vendor::IntelArc)),
            _ => false,
        }
    }

    /// `/dev/dri/by-path/pci-<addr>-render`: stable across boots, unlike `renderD128`/`renderD129`.
    pub fn render_node(&self) -> String {
        format!("/dev/dri/by-path/pci-{}-render", self.pci_addr)
    }

    pub fn card_node(&self) -> String {
        format!("/dev/dri/by-path/pci-{}-card", self.pci_addr)
    }
}

/// Parses `lspci -nn -D`: one [`GpuDevice`] per VGA / 3D / display controller line.
pub fn parse_lspci(text: &str) -> Vec<GpuDevice> {
    text.lines()
        .filter_map(|line| {
            let (addr, rest) = line.split_once(' ')?;
            // `[0300]` VGA, `[0302]` 3D controller, `[0380]` other display controller.
            if !(rest.contains("[0300]") || rest.contains("[0302]") || rest.contains("[0380]")) || !addr.contains(':') {
                return None;
            }
            let id_start = rest.rfind("] [").or_else(|| rest.rfind(" ["))?; // …[vendor:device] (rev xx)
            let ids = rest[id_start..].split(['[', ']']).find(|s| s.len() == 9 && s.as_bytes().get(4) == Some(&b':'))?;
            let (vendor_id, device_id) = ids.split_once(':')?;
            let vendor = match vendor_id.to_ascii_lowercase().as_str() {
                "10de" => Vendor::Nvidia,
                "1002" => Vendor::Amd,
                "8086" => {
                    let d = device_id.to_ascii_lowercase();
                    if d.starts_with("56") || d.starts_with("e2") { Vendor::IntelArc } else { Vendor::Intel }
                }
                "1af4" => Vendor::Virtio,
                "15ad" => Vendor::Vmware,
                "1b36" => Vendor::Qxl,
                _ => Vendor::Other,
            };
            // `0000:01:00.0` → keep the domain; older lspci prints `01:00.0` without it.
            let pci_addr = if addr.matches(':').count() == 1 { format!("0000:{addr}") } else { addr.to_string() };
            Some(GpuDevice { vendor, pci_addr, device_id: device_id.to_ascii_lowercase(), description: rest.trim().to_string() })
        })
        .collect()
}

/// Mesa's `VIDEO_CARDS` values for the adapters in `devices`, plus `extra` (a GPU the user chose by hand),
/// most capable first, without repeats. `nouveau` selects the open NVIDIA driver instead of the proprietary one.
pub fn video_cards(devices: &[GpuDevice], extra: Option<Gpu>, nouveau: bool) -> Vec<&'static str> {
    let mut order: Vec<&GpuDevice> = devices.iter().collect();
    order.sort_by_key(|d| std::cmp::Reverse(d.vendor.rank()));

    let mut out: Vec<&'static str> = Vec::new();
    let mut push = |values: &[&'static str]| {
        for v in values {
            if !out.contains(v) {
                out.push(v);
            }
        }
    };
    for d in order {
        match d.vendor {
            Vendor::Nvidia => push(if nouveau { &["nouveau"] } else { &["nvidia"] }),
            Vendor::Amd => push(&["amdgpu", "radeonsi"]),
            Vendor::Intel | Vendor::IntelArc => push(&["intel"]),
            Vendor::Virtio => push(&["virgl"]),
            Vendor::Vmware => push(&["vmware"]),
            Vendor::Qxl => push(&["qxl"]),
            Vendor::Other => {}
        }
    }
    match extra {
        Some(Gpu::Nvidia) => push(&["nvidia"]),
        Some(Gpu::Nouveau) => push(&["nouveau"]),
        Some(Gpu::Amd) => push(&["amdgpu", "radeonsi"]),
        Some(Gpu::RadeonLegacy) => push(&["radeon", "r300", "r600"]),
        Some(Gpu::Intel | Gpu::Xe) => push(&["intel"]),
        Some(Gpu::None) | None => {}
    }
    out
}

/// The adapter the compositor should render on. Physical adapters are preferred over virtual ones;
/// among those, [`RenderPreference::Auto`] takes the most capable (discrete) one and
/// [`RenderPreference::Integrated`] the least capable. `None` when the machine has no adapter.
pub fn pick_render(devices: &[GpuDevice], pref: RenderPreference) -> Option<&GpuDevice> {
    let physical: Vec<&GpuDevice> = devices.iter().filter(|d| !d.vendor.is_virtual()).collect();
    let pool: Vec<&GpuDevice> = if physical.is_empty() { devices.iter().collect() } else { physical };
    match pref {
        RenderPreference::Auto => pool.into_iter().max_by_key(|d| d.vendor.rank()),
        RenderPreference::Integrated => pool.into_iter().min_by_key(|d| d.vendor.rank()),
    }
}

/// Whether the compositor will use the proprietary NVIDIA driver for the chosen render device (it then
/// needs kernel modesetting, and Noctalia a private GL context).
pub fn renders_on_proprietary_nvidia(chosen: Option<&GpuDevice>, nouveau: bool) -> bool {
    chosen.is_some_and(|d| d.vendor == Vendor::Nvidia) && !nouveau
}

/// What the desktop setup does with the chosen render device.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RenderPlan {
    /// `/dev/dri/by-path/…-render` of the adapter to render on; set only when the machine has more than one
    /// physical adapter (with one there is nothing to choose, and a pinned path would only break on a swapped card).
    pub render_node: Option<String>,
    /// The matching `…-card` node, for `WLR_DRM_DEVICES` / `AQ_DRM_DEVICES`.
    pub card_node: Option<String>,
    /// Rendering on the proprietary NVIDIA driver: Noctalia must not share one GL context across its surfaces.
    pub proprietary_nvidia: bool,
}

impl RenderPlan {
    pub fn new(devices: &[GpuDevice], pref: RenderPreference, nouveau: bool) -> RenderPlan {
        let chosen = pick_render(devices, pref);
        let physical = devices.iter().filter(|d| !d.vendor.is_virtual()).count();
        RenderPlan {
            render_node: chosen.filter(|_| physical > 1).map(GpuDevice::render_node),
            card_node: chosen.filter(|_| physical > 1).map(GpuDevice::card_node),
            proprietary_nvidia: renders_on_proprietary_nvidia(chosen, nouveau),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real `lspci -nn -D` lines (this machine's, and the virtual adapters QEMU/VMware/Hyper-V show).
    const HYBRID: &str = "\
0000:00:02.0 VGA compatible controller [0300]: Intel Corporation Raptor Lake-S UHD Graphics [8086:a78b] (rev 04)
0000:00:1f.3 Audio device [0403]: Intel Corporation Device [8086:7a50] (rev 11)
0000:01:00.0 VGA compatible controller [0300]: NVIDIA Corporation AD106M [GeForce RTX 4070 Max-Q / Mobile] [10de:28e1] (rev a1)
";
    const VIRTIO: &str = "0000:00:02.0 VGA compatible controller [0300]: Red Hat, Inc. Virtio 1.0 GPU [1af4:1050] (rev 01)\n";

    #[test]
    fn parses_the_adapters_and_ignores_everything_else() {
        let d = parse_lspci(HYBRID);
        assert_eq!(d.len(), 2);
        assert_eq!((d[0].vendor, d[0].pci_addr.as_str(), d[0].device_id.as_str()), (Vendor::Intel, "0000:00:02.0", "a78b"));
        assert_eq!((d[1].vendor, d[1].pci_addr.as_str()), (Vendor::Nvidia, "0000:01:00.0"));
        assert!(parse_lspci("").is_empty() && parse_lspci("garbage\n[0300]\n").is_empty());
    }

    #[test]
    fn an_address_without_the_domain_gets_one() {
        let d = parse_lspci("01:00.0 3D controller [0302]: NVIDIA Corporation GA107 [10de:25a0]\n");
        assert_eq!(d[0].pci_addr, "0000:01:00.0");
        assert_eq!(d[0].render_node(), "/dev/dri/by-path/pci-0000:01:00.0-render");
    }

    #[test]
    fn a_hybrid_laptop_gets_a_driver_for_both_adapters_discrete_first() {
        // The old code wrote only `nvidia`, leaving Mesa without a driver for the iGPU the panel hangs off.
        assert_eq!(video_cards(&parse_lspci(HYBRID), None, false), ["nvidia", "intel"]);
        assert_eq!(video_cards(&parse_lspci(HYBRID), None, true), ["nouveau", "intel"]);
    }

    #[test]
    fn gentoo_driver_names_not_kernel_module_names() {
        let amd = parse_lspci("0000:04:00.0 VGA compatible controller [0300]: Advanced Micro Devices, Inc. [AMD/ATI] Phoenix1 [1002:15bf]\n");
        assert_eq!(video_cards(&amd, None, false), ["amdgpu", "radeonsi"]);
        let arc = parse_lspci("0000:03:00.0 VGA compatible controller [0300]: Intel Corporation DG2 [Arc A770] [8086:56a0]\n");
        assert_eq!(arc[0].vendor, Vendor::IntelArc);
        assert_eq!(video_cards(&arc, None, false), ["intel"], "Arc is `intel`, not `xe` or `i915`");
    }

    #[test]
    fn a_virtual_gpu_gets_its_mesa_driver() {
        assert_eq!(video_cards(&parse_lspci(VIRTIO), None, false), ["virgl"]);
        assert_eq!(video_cards(&[], None, false), Vec::<&str>::new());
    }

    #[test]
    fn a_hand_picked_gpu_adds_to_the_detected_ones() {
        assert_eq!(video_cards(&parse_lspci(VIRTIO), Some(Gpu::Amd), false), ["virgl", "amdgpu", "radeonsi"]);
    }

    #[test]
    fn the_discrete_gpu_renders_unless_the_integrated_one_is_asked_for() {
        let d = parse_lspci(HYBRID);
        assert_eq!(pick_render(&d, RenderPreference::Auto).unwrap().vendor, Vendor::Nvidia);
        assert_eq!(pick_render(&d, RenderPreference::Integrated).unwrap().vendor, Vendor::Intel);
        assert!(pick_render(&[], RenderPreference::Auto).is_none());
    }

    #[test]
    fn a_physical_adapter_beats_a_virtual_one_and_amd_is_discrete_only_next_to_an_intel_igpu() {
        let mixed = parse_lspci(&format!("{VIRTIO}0000:01:00.0 VGA compatible controller [0300]: AMD/ATI Navi 23 [1002:73ff]\n0000:00:02.0 VGA compatible controller [0300]: Intel Raptor Lake [8086:a780]\n"));
        assert_eq!(pick_render(&mixed, RenderPreference::Auto).unwrap().vendor, Vendor::Amd);
        let amd = mixed.iter().find(|d| d.vendor == Vendor::Amd).unwrap();
        assert!(amd.is_discrete(&mixed));
        let apu_plus_nvidia = parse_lspci("0000:05:00.0 VGA [0300]: AMD Phoenix [1002:15bf]\n0000:01:00.0 VGA [0300]: NVIDIA AD107 [10de:28a0]\n");
        let apu = apu_plus_nvidia.iter().find(|d| d.vendor == Vendor::Amd).unwrap();
        assert!(!apu.is_discrete(&apu_plus_nvidia), "an AMD APU next to an NVIDIA card is the integrated one");
    }

    #[test]
    fn proprietary_nvidia_is_flagged_only_when_the_render_device_is_nvidia_and_not_nouveau() {
        let d = parse_lspci(HYBRID);
        let nv = pick_render(&d, RenderPreference::Auto);
        assert!(renders_on_proprietary_nvidia(nv, false));
        assert!(!renders_on_proprietary_nvidia(nv, true));
        assert!(!renders_on_proprietary_nvidia(pick_render(&d, RenderPreference::Integrated), false));
    }

    #[test]
    fn the_plan_pins_a_device_only_on_a_multi_gpu_machine() {
        let hybrid = RenderPlan::new(&parse_lspci(HYBRID), RenderPreference::Auto, false);
        assert_eq!(hybrid.render_node.as_deref(), Some("/dev/dri/by-path/pci-0000:01:00.0-render"));
        assert_eq!(hybrid.card_node.as_deref(), Some("/dev/dri/by-path/pci-0000:01:00.0-card"));
        assert!(hybrid.proprietary_nvidia);
        let igpu = RenderPlan::new(&parse_lspci(HYBRID), RenderPreference::Integrated, false);
        assert_eq!(igpu.render_node.as_deref(), Some("/dev/dri/by-path/pci-0000:00:02.0-render"));
        assert!(!igpu.proprietary_nvidia);
        assert_eq!(RenderPlan::new(&parse_lspci(VIRTIO), RenderPreference::Auto, false), RenderPlan::default());
    }
}
