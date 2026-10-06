//! Apple model database (subset relevant to Hackintosh SMBIOS choices),
//! from acidanthera `AppleModels/DataBase` (master 2026-09-30).
//!
//! Every Intel Mac whose last supported release is High Sierra or newer is
//! listed. `min_release`/`max_release` are the database's Minimum/Maximum OS
//! versions; `secure_boot_model` is its lowercased `AppleModelId` (T2 models).

use super::model::MacOsVersion;
use MacKind::{Desktop, Laptop, Workstation};
use MacOsVersion::{
    BigSur, Catalina, HighSierra, Mojave, Monterey, Sequoia, Sonoma, Tahoe, Ventura,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacKind {
    Desktop,
    Laptop,
    Workstation,
}

#[derive(Debug, Clone, Copy)]
pub struct SmbiosModel {
    pub model: &'static str,
    pub board_id: &'static str,
    /// Oldest macOS (from High Sierra on) this model can install; None = older than 10.13.
    pub min_os: Option<MacOsVersion>,
    /// Newest macOS this model is supported by.
    pub max_os: MacOsVersion,
    /// `AppleModelId` (T2 Secure Boot model, e.g. "j185"); None = x86legacy.
    pub secure_boot_model: Option<&'static str>,
    pub kind: MacKind,
    /// Short description of the real hardware ("iMac 27\" 2020, Comet Lake").
    pub description: &'static str,
    /// First macOS point release shipped for this model ("10.15.6").
    pub min_release: &'static str,
    /// Last macOS point release Apple supports on this model ("26.7.1").
    pub max_release: &'static str,
}

#[allow(clippy::too_many_arguments)]
const fn m(
    model: &'static str,
    board_id: &'static str,
    min_release: &'static str,
    max_release: &'static str,
    min_os: Option<MacOsVersion>,
    max_os: MacOsVersion,
    secure_boot_model: Option<&'static str>,
    kind: MacKind,
    description: &'static str,
) -> SmbiosModel {
    SmbiosModel {
        model,
        board_id,
        min_os,
        max_os,
        secure_boot_model,
        kind,
        description,
        min_release,
        max_release,
    }
}

#[rustfmt::skip]
static MODELS: &[SmbiosModel] = &[
    // MacBook
    m("MacBook6,1", "Mac-F22C8AC8", "10.6.1", "10.13.6", None, HighSierra, None, Laptop, "MacBook 13\" 2009, Core 2 Duo, GeForce 9400M"),
    m("MacBook7,1", "Mac-F22C89C8", "10.6.3", "10.13.6", None, HighSierra, None, Laptop, "MacBook 13\" 2010, Core 2 Duo, GeForce 320M"),
    m("MacBook8,1", "Mac-BE0E8AC46FE800CC", "10.10.2", "11.7.11", None, BigSur, None, Laptop, "MacBook 12\" 2015, Core M (Broadwell)"),
    m("MacBook9,1", "Mac-9AE82516C7C6B903", "10.11.4", "12.7.6", None, Monterey, None, Laptop, "MacBook 12\" 2016, Core m3/m5/m7 (Skylake)"),
    m("MacBook10,1", "Mac-EE2EBD4B90B839A8", "10.12.5", "13.7.8", None, Ventura, None, Laptop, "MacBook 12\" 2017, Core m3/i5/i7 (Kaby Lake)"),
    // MacBook Air
    m("MacBookAir3,1", "Mac-942452F5819B1C1B", "10.6.4", "10.13.6", None, HighSierra, None, Laptop, "MacBook Air 11\" 2010, Core 2 Duo, GeForce 320M"),
    m("MacBookAir3,2", "Mac-942C5DF58193131B", "10.6.4", "10.13.6", None, HighSierra, None, Laptop, "MacBook Air 13\" 2010, Core 2 Duo, GeForce 320M"),
    m("MacBookAir4,1", "Mac-C08A6BB70A942AC2", "10.7", "10.13.6", None, HighSierra, None, Laptop, "MacBook Air 11\" 2011, Sandy Bridge, HD 3000"),
    m("MacBookAir4,2", "Mac-742912EFDBEE19B3", "10.7.3", "10.13.6", None, HighSierra, None, Laptop, "MacBook Air 13\" 2011, Sandy Bridge, HD 3000"),
    m("MacBookAir5,1", "Mac-66F35F19FE2A0D05", "10.7.4", "10.15.8", None, Catalina, None, Laptop, "MacBook Air 11\" 2012, Ivy Bridge, HD 4000"),
    m("MacBookAir5,2", "Mac-2E6FAB96566FE58C", "10.8.2", "10.15.8", None, Catalina, None, Laptop, "MacBook Air 13\" 2012, Ivy Bridge, HD 4000"),
    m("MacBookAir6,1", "Mac-35C1E88140C3E6CF", "10.9.2", "11.7.11", None, BigSur, None, Laptop, "MacBook Air 11\" 2013-2014, Haswell, HD 5000"),
    m("MacBookAir6,2", "Mac-7DF21CB3ED6977E5", "10.9.2", "11.7.11", None, BigSur, None, Laptop, "MacBook Air 13\" 2013-2014, Haswell, HD 5000"),
    m("MacBookAir7,1", "Mac-9F18E312C5C2BF0B", "10.10.2", "12.7.6", None, Monterey, None, Laptop, "MacBook Air 11\" 2015, Broadwell, HD 6000"),
    m("MacBookAir7,2", "Mac-937CB26E2E02BB01", "10.12.5", "12.7.6", None, Monterey, None, Laptop, "MacBook Air 13\" 2015-2017, Broadwell, HD 6000"),
    m("MacBookAir8,1", "Mac-827FAC58A8FDFA22", "10.14.1", "14.8.9", Some(Mojave), Sonoma, Some("j140k"), Laptop, "MacBook Air 13\" 2018, Amber Lake, UHD 617"),
    m("MacBookAir8,2", "Mac-226CB3C6A851A671", "10.14.5", "14.8.9", Some(Mojave), Sonoma, Some("j140a"), Laptop, "MacBook Air 13\" 2019, Amber Lake, UHD 617"),
    m("MacBookAir9,1", "Mac-0CFF9C7C2B63DF8D", "10.15.3", "15.8.1", Some(Catalina), Sequoia, Some("j230k"), Laptop, "MacBook Air 13\" 2020, Ice Lake, Iris Plus"),
    // MacBook Pro
    m("MacBookPro6,1", "Mac-F22589C8", "10.6.3", "10.13.6", None, HighSierra, None, Laptop, "MacBook Pro 17\" 2010, Arrandale, GeForce GT 330M"),
    m("MacBookPro6,2", "Mac-F22586C8", "10.6.3", "10.13.6", None, HighSierra, None, Laptop, "MacBook Pro 15\" 2010, Arrandale, GeForce GT 330M"),
    m("MacBookPro7,1", "Mac-F222BEC8", "10.6.3", "10.13.6", None, HighSierra, None, Laptop, "MacBook Pro 13\" 2010, Core 2 Duo, GeForce 320M"),
    m("MacBookPro8,1", "Mac-94245B3640C91C81", "10.7.2", "10.13.6", None, HighSierra, None, Laptop, "MacBook Pro 13\" 2011, Sandy Bridge, HD 3000"),
    m("MacBookPro8,2", "Mac-94245A3940C91C80", "10.7.2", "10.13.6", None, HighSierra, None, Laptop, "MacBook Pro 15\" 2011, Sandy Bridge quad-core, Radeon HD 6490M/6750M"),
    m("MacBookPro8,3", "Mac-942459F5819B171B", "10.7.2", "10.13.6", None, HighSierra, None, Laptop, "MacBook Pro 17\" 2011, Sandy Bridge quad-core, Radeon HD 6750M/6770M"),
    m("MacBookPro9,1", "Mac-4B7AC7E43945597E", "10.7.3", "10.15.8", None, Catalina, None, Laptop, "MacBook Pro 15\" 2012, Ivy Bridge quad-core, GeForce GT 650M"),
    m("MacBookPro9,2", "Mac-6F01561E16C75D06", "10.7.3", "10.15.8", None, Catalina, None, Laptop, "MacBook Pro 13\" 2012, Ivy Bridge, HD 4000"),
    m("MacBookPro10,1", "Mac-C3EC7CD22292981F", "10.8.2", "10.15.8", None, Catalina, None, Laptop, "MacBook Pro 15\" Retina 2012-2013, Ivy Bridge quad-core, GeForce GT 650M"),
    m("MacBookPro10,2", "Mac-AFD8A9D944EA4843", "10.8.2", "10.15.8", None, Catalina, None, Laptop, "MacBook Pro 13\" Retina 2012-2013, Ivy Bridge, HD 4000"),
    m("MacBookPro11,1", "Mac-189A3D4F975D5FFC", "10.9.4", "11.7.11", None, BigSur, None, Laptop, "MacBook Pro 13\" Retina 2013-2014, Haswell, Iris 5100"),
    m("MacBookPro11,2", "Mac-3CBD00234E554E41", "10.9.4", "11.7.11", None, BigSur, None, Laptop, "MacBook Pro 15\" Retina 2013-2014, Haswell quad-core, Iris Pro 5200"),
    m("MacBookPro11,3", "Mac-2BD1B31983FE1663", "10.9.4", "11.7.11", None, BigSur, None, Laptop, "MacBook Pro 15\" Retina 2013-2014, Haswell quad-core, GeForce GT 750M"),
    m("MacBookPro11,4", "Mac-06F11FD93F0323C5", "10.10.3", "12.7.6", None, Monterey, None, Laptop, "MacBook Pro 15\" 2015, Haswell quad-core, Iris Pro 5200"),
    m("MacBookPro11,5", "Mac-06F11F11946D27C5", "10.10.3", "12.7.6", None, Monterey, None, Laptop, "MacBook Pro 15\" 2015, Haswell quad-core, Radeon R9 M370X"),
    m("MacBookPro12,1", "Mac-E43C1C25D4880AD6", "10.10.2", "12.7.6", None, Monterey, None, Laptop, "MacBook Pro 13\" 2015, Broadwell, Iris 6100"),
    m("MacBookPro13,1", "Mac-473D31EABEB93F9B", "10.12", "12.7.6", None, Monterey, None, Laptop, "MacBook Pro 13\" 2016, two Thunderbolt 3 ports, Skylake, Iris 540"),
    m("MacBookPro13,2", "Mac-66E35819EE2D0D05", "10.12.1", "12.7.6", None, Monterey, None, Laptop, "MacBook Pro 13\" 2016, four Thunderbolt 3 ports, Skylake, Iris 550"),
    m("MacBookPro13,3", "Mac-A5C67F76ED83108C", "10.12.1", "12.7.6", None, Monterey, None, Laptop, "MacBook Pro 15\" 2016, Skylake quad-core, Radeon Pro 450/455/460"),
    m("MacBookPro14,1", "Mac-B4831CEBD52A0C4C", "10.12.5", "13.7.8", None, Ventura, None, Laptop, "MacBook Pro 13\" 2017, two Thunderbolt 3 ports, Kaby Lake, Iris Plus 640"),
    m("MacBookPro14,2", "Mac-CAD6701F7CEA0921", "10.12.5", "13.7.8", None, Ventura, None, Laptop, "MacBook Pro 13\" 2017, four Thunderbolt 3 ports, Kaby Lake, Iris Plus 650"),
    m("MacBookPro14,3", "Mac-551B86E5744E2388", "10.12.5", "13.7.8", None, Ventura, None, Laptop, "MacBook Pro 15\" 2017, Kaby Lake quad-core, Radeon Pro 555/560"),
    m("MacBookPro15,1", "Mac-937A206F2EE63C01", "10.13.6", "15.8.1", Some(HighSierra), Sequoia, Some("j680"), Laptop, "MacBook Pro 15\" 2018-2019, Coffee Lake-H, Radeon Pro 555X/560X"),
    m("MacBookPro15,2", "Mac-827FB448E656EC26", "10.13.6", "15.8.1", Some(HighSierra), Sequoia, Some("j132"), Laptop, "MacBook Pro 13\" 2018-2019, four Thunderbolt 3 ports, Coffee Lake-U, Iris Plus 655"),
    m("MacBookPro15,3", "Mac-1E7E29AD0135F9BC", "10.14.5", "15.8.1", Some(Mojave), Sequoia, Some("j780"), Laptop, "MacBook Pro 15\" 2018-2019, Coffee Lake-H, Radeon Pro Vega 16/20"),
    m("MacBookPro15,4", "Mac-53FDB3D8DB8CA971", "10.14.5", "15.8.1", Some(Mojave), Sequoia, Some("j213"), Laptop, "MacBook Pro 13\" 2019, two Thunderbolt 3 ports, Coffee Lake-U, Iris Plus 645"),
    m("MacBookPro16,1", "Mac-E1008331FDC96864", "10.15.1", "26.7.1", Some(Catalina), Tahoe, Some("j152f"), Laptop, "MacBook Pro 16\" 2019, Coffee Lake-H, Radeon Pro 5300M/5500M"),
    m("MacBookPro16,2", "Mac-5F9802EFE386AA28", "10.15.4", "26.7.1", Some(Catalina), Tahoe, Some("j214k"), Laptop, "MacBook Pro 13\" 2020, four Thunderbolt 3 ports, Ice Lake, Iris Plus G7"),
    m("MacBookPro16,3", "Mac-E7203C0F68AA0004", "10.15.4", "15.8.1", Some(Catalina), Sequoia, Some("j223"), Laptop, "MacBook Pro 13\" 2020, two Thunderbolt 3 ports, Coffee Lake-U, Iris Plus 645"),
    m("MacBookPro16,4", "Mac-A61BADE1FDAD7B05", "10.15.5", "26.7.1", Some(Catalina), Tahoe, Some("j215"), Laptop, "MacBook Pro 16\" 2019, Coffee Lake-H, Radeon Pro 5600M"),
    // Mac mini
    m("Macmini4,1", "Mac-F2208EC8", "10.6.4", "10.13.6", None, HighSierra, None, Desktop, "Mac mini 2010, Core 2 Duo, GeForce 320M"),
    m("Macmini5,1", "Mac-8ED6AF5B48C039E1", "10.7", "10.13.6", None, HighSierra, None, Desktop, "Mac mini 2011, Sandy Bridge dual-core, HD 3000"),
    m("Macmini5,2", "Mac-4BC72D62AD45599E", "10.7", "10.13.6", None, HighSierra, None, Desktop, "Mac mini 2011, Sandy Bridge dual-core, Radeon HD 6630M"),
    m("Macmini5,3", "Mac-7BA5B2794B2CDB12", "10.7", "10.13.6", None, HighSierra, None, Desktop, "Mac mini Server 2011, Sandy Bridge quad-core, HD 3000"),
    m("Macmini6,1", "Mac-031AEE4D24BFF0B1", "10.8.1", "10.15.8", None, Catalina, None, Desktop, "Mac mini 2012, Ivy Bridge dual-core, HD 4000"),
    m("Macmini6,2", "Mac-F65AE981FFA204ED", "10.8.2", "10.15.8", None, Catalina, None, Desktop, "Mac mini 2012, Ivy Bridge quad-core, HD 4000"),
    m("Macmini7,1", "Mac-35C5E08120C7EEAF", "10.10", "12.7.6", None, Monterey, None, Desktop, "Mac mini 2014, Haswell, HD 5000/Iris 5100"),
    m("Macmini8,1", "Mac-7BA5B2DFE22DDD8C", "10.14", "15.8.1", Some(Mojave), Sequoia, Some("j174"), Desktop, "Mac mini 2018, Coffee Lake, UHD 630"),
    // Mac Pro and iMac Pro
    m("MacPro5,1", "Mac-F221BEC8", "10.7.4", "10.14.6", None, Mojave, None, Workstation, "Mac Pro 2010-2012, Westmere Xeon"),
    m("MacPro6,1", "Mac-F60DEB81FF30ACF6", "10.9.1", "12.7.6", None, Monterey, None, Workstation, "Mac Pro 2013, Ivy Bridge-EP Xeon E5 v2, FirePro D300/D500/D700"),
    m("MacPro7,1", "Mac-27AD2F918AE68F61", "10.15.1", "26.7.1", Some(Catalina), Tahoe, Some("j160"), Workstation, "Mac Pro 2019, Cascade Lake-W Xeon W, no iGPU"),
    m("iMacPro1,1", "Mac-7BA5B2D9E42DDD94", "10.13.2", "15.8.1", Some(HighSierra), Sequoia, Some("j137"), Workstation, "iMac Pro 2017, Skylake-W Xeon W, Radeon Pro Vega 56/64, no iGPU"),
    // iMac
    m("iMac10,1", "Mac-F2268CC8", "10.6.1", "10.13.6", None, HighSierra, None, Desktop, "iMac 21.5\"/27\" 2009, Core 2 Duo, GeForce 9400M/Radeon HD 4670"),
    m("iMac11,1", "Mac-F2268DAE", "10.6.2", "10.13.6", None, HighSierra, None, Desktop, "iMac 27\" 2009, Lynnfield Core i5/i7, Radeon HD 4850"),
    m("iMac11,2", "Mac-F2238AC8", "10.6.3", "10.13.6", None, HighSierra, None, Desktop, "iMac 21.5\" 2010, Clarkdale Core i3/i5, Radeon HD 4670/5670"),
    m("iMac11,3", "Mac-F2238BAE", "10.6.3", "10.13.6", None, HighSierra, None, Desktop, "iMac 27\" 2010, Clarkdale/Lynnfield, Radeon HD 5670/5750"),
    m("iMac12,1", "Mac-942B5BF58194151B", "10.7.2", "10.13.6", None, HighSierra, None, Desktop, "iMac 21.5\" 2011, Sandy Bridge, Radeon HD 6750M/6770M"),
    m("iMac12,2", "Mac-942B59F58194171B", "10.6.6", "10.13.6", None, HighSierra, None, Desktop, "iMac 27\" 2011, Sandy Bridge, Radeon HD 6770M/6970M"),
    m("iMac13,1", "Mac-00BE6ED71E35EB86", "10.8.2", "10.15.8", None, Catalina, None, Desktop, "iMac 21.5\" 2012, Ivy Bridge, GeForce GT 640M/650M"),
    m("iMac13,2", "Mac-FC02E91DDD3FA6A4", "10.8.2", "10.15.8", None, Catalina, None, Desktop, "iMac 27\" 2012, Ivy Bridge, GeForce GTX 660M-680MX"),
    m("iMac13,3", "Mac-7DF2A3B5E5D671ED", "10.8.2", "10.15.8", None, Catalina, None, Desktop, "iMac 21.5\" 2013 (education), Ivy Bridge, HD 4000"),
    m("iMac14,1", "Mac-031B6874CF7F642A", "10.8.4", "10.15.8", None, Catalina, None, Desktop, "iMac 21.5\" 2013, Haswell, Iris Pro 5200"),
    m("iMac14,2", "Mac-27ADBB7B4CEE8E61", "10.8.4", "10.15.8", None, Catalina, None, Desktop, "iMac 27\" 2013, Haswell, GeForce GT 755M/GTX 775M/780M"),
    m("iMac14,3", "Mac-77EB7D7DAF985301", "10.8.4", "10.15.8", None, Catalina, None, Desktop, "iMac 21.5\" 2013, Haswell, GeForce GT 750M"),
    m("iMac14,4", "Mac-81E3E92DD6088272", "10.9.3", "11.7.11", None, BigSur, None, Desktop, "iMac 21.5\" 2014, Haswell, HD 5000"),
    m("iMac15,1", "Mac-42FD25EABCABB274", "10.10.2", "11.7.11", None, BigSur, None, Desktop, "iMac 27\" Retina 5K 2014-2015, Haswell, Radeon R9 M290X/M295X"),
    m("iMac16,1", "Mac-A369DDC4E67F1C45", "10.11", "12.7.6", None, Monterey, None, Desktop, "iMac 21.5\" 2015, Broadwell, Iris 6100"),
    m("iMac16,2", "Mac-FFE5EF870D7BA81A", "10.11", "12.7.6", None, Monterey, None, Desktop, "iMac 21.5\" Retina 4K 2015, Broadwell, Iris Pro 6200"),
    m("iMac17,1", "Mac-DB15BD556843C820", "10.11", "12.7.6", None, Monterey, None, Desktop, "iMac 27\" Retina 5K 2015, Skylake, Radeon R9 M380/M390/M395"),
    m("iMac18,1", "Mac-4B682C642B45593E", "10.12.4", "13.7.8", None, Ventura, None, Desktop, "iMac 21.5\" 2017, Kaby Lake, Iris Plus 640"),
    m("iMac18,2", "Mac-77F17D7DA9285301", "10.12.4", "13.7.8", None, Ventura, None, Desktop, "iMac 21.5\" Retina 4K 2017, Kaby Lake, Radeon Pro 555/560"),
    m("iMac18,3", "Mac-BE088AF8C5EB4FA2", "10.12.4", "13.7.8", None, Ventura, None, Desktop, "iMac 27\" Retina 5K 2017, Kaby Lake, Radeon Pro 570/575/580"),
    m("iMac19,1", "Mac-AA95B1DDAB278B95", "10.14.4", "15.8.1", Some(Mojave), Sequoia, None, Desktop, "iMac 27\" Retina 5K 2019, Coffee Lake, Radeon Pro 570X-Vega 48"),
    m("iMac19,2", "Mac-63001698E7A34814", "10.14.4", "15.8.1", Some(Mojave), Sequoia, None, Desktop, "iMac 21.5\" Retina 4K 2019, Coffee Lake, Radeon Pro 555X/560X/Vega 20"),
    m("iMac20,1", "Mac-CFF7D910A743CAAF", "10.15.6", "26.7.1", Some(Catalina), Tahoe, Some("j185"), Desktop, "iMac 27\" 2020, Comet Lake up to 8 cores, Radeon Pro 5300/5500 XT"),
    m("iMac20,2", "Mac-AF89B6D9451A490B", "10.15.6", "26.7.1", Some(Catalina), Tahoe, Some("j185f"), Desktop, "iMac 27\" 2020, Comet Lake i9 10-core, Radeon Pro 5500 XT/5700/5700 XT"),
];

/// Every model the planner may choose or the UI may offer.
pub fn all() -> &'static [SmbiosModel] {
    MODELS
}

pub fn find(model: &str) -> Option<&'static SmbiosModel> {
    all().iter().find(|m| m.model.eq_ignore_ascii_case(model))
}

/// Board-ids some models shipped with besides the primary [`SmbiosModel::board_id`].
static ALT_BOARD_IDS: &[(&str, &str)] = &[
    ("Mac-F221DCC8", "iMac10,1"),
    ("Mac-65CE76090165799A", "iMac17,1"),
    ("Mac-B809C3757DA9BB8D", "iMac17,1"),
];

/// Look up a model by board-id ("Mac-CFF7D910A743CAAF"), including the
/// secondary board-ids of iMac10,1 and iMac17,1.
pub fn find_by_board_id(board_id: &str) -> Option<&'static SmbiosModel> {
    let id = board_id.trim();
    all()
        .iter()
        .find(|m| m.board_id.eq_ignore_ascii_case(id))
        .or_else(|| {
            ALT_BOARD_IDS
                .iter()
                .find(|(alt, _)| alt.eq_ignore_ascii_case(id))
                .and_then(|(_, model)| find(model))
        })
}

/// True when Apple supports `version` on `model` (no board-id skip needed).
pub fn supports(model: &str, version: MacOsVersion) -> bool {
    find(model)
        .is_some_and(|m| m.max_os >= version && !matches!(m.min_os, Some(min) if min > version))
}

/// All models Apple supports on `version`, in table order.
pub fn models_supporting(version: MacOsVersion) -> Vec<&'static SmbiosModel> {
    all()
        .iter()
        .filter(|m| supports(m.model, version))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(version: MacOsVersion) -> Vec<&'static str> {
        let mut v: Vec<&str> = models_supporting(version).iter().map(|m| m.model).collect();
        v.sort_unstable();
        v
    }

    fn sorted(list: &[&'static str]) -> Vec<&'static str> {
        let mut v = list.to_vec();
        v.sort_unstable();
        v
    }

    #[test]
    fn table_size_and_uniqueness() {
        assert_eq!(all().len(), 84);
        let mut models: Vec<&str> = all().iter().map(|m| m.model).collect();
        models.sort_unstable();
        models.dedup();
        assert_eq!(models.len(), 84, "duplicate model ids");
        for m in all() {
            assert!(m.board_id.starts_with("Mac-"), "{}", m.model);
            assert!(!m.description.is_empty());
            assert!(m.max_os >= HighSierra);
        }
    }

    #[test]
    fn releases_match_major_versions() {
        for m in all() {
            assert_eq!(
                MacOsVersion::parse(m.max_release),
                Some(m.max_os),
                "{} max",
                m.model
            );
            assert_eq!(
                MacOsVersion::parse(m.min_release),
                m.min_os,
                "{} min",
                m.model
            );
        }
    }

    #[test]
    fn tahoe_list() {
        assert_eq!(
            names(Tahoe),
            sorted(&[
                "MacBookPro16,1",
                "MacBookPro16,2",
                "MacBookPro16,4",
                "MacPro7,1",
                "iMac20,1",
                "iMac20,2"
            ])
        );
    }

    #[test]
    fn sequoia_list() {
        let mut want = vec![
            "MacBookAir9,1",
            "MacBookPro15,1",
            "MacBookPro15,2",
            "MacBookPro15,3",
            "MacBookPro15,4",
            "MacBookPro16,3",
            "Macmini8,1",
            "iMac19,1",
            "iMac19,2",
            "iMacPro1,1",
        ];
        want.extend(names(Tahoe));
        assert_eq!(names(Sequoia), sorted(&want));
        assert_eq!(names(Sequoia).len(), 16);
    }

    #[test]
    fn per_version_counts_match_database() {
        let counts = [
            (Sonoma, 18),
            (Ventura, 25),
            (Monterey, 39),
            (BigSur, 47),
            (Catalina, 61),
            (Mojave, 54),
            (HighSierra, 69),
        ];
        for (version, count) in counts {
            assert_eq!(models_supporting(version).len(), count, "{version:?}");
        }
    }

    #[test]
    fn version_deltas() {
        let sonoma = names(Sonoma);
        let sequoia = names(Sequoia);
        let added: Vec<&str> = sonoma
            .iter()
            .filter(|m| !sequoia.contains(m))
            .copied()
            .collect();
        assert_eq!(added, vec!["MacBookAir8,1", "MacBookAir8,2"]);

        let ventura = names(Ventura);
        let added: Vec<&str> = ventura
            .iter()
            .filter(|m| !sonoma.contains(m))
            .copied()
            .collect();
        assert_eq!(
            added,
            sorted(&[
                "MacBook10,1",
                "MacBookPro14,1",
                "MacBookPro14,2",
                "MacBookPro14,3",
                "iMac18,1",
                "iMac18,2",
                "iMac18,3"
            ])
        );

        let mojave = names(Mojave);
        assert!(mojave.contains(&"MacPro5,1"));
        assert!(!mojave.contains(&"iMac20,1"));
        assert!(!names(Catalina).contains(&"MacPro5,1"));
    }

    #[test]
    fn supports_matrix() {
        let matrix: &[(&str, MacOsVersion, bool)] = &[
            ("iMac20,1", Tahoe, true),
            ("iMac20,1", Catalina, true),
            ("iMac20,1", Mojave, false),
            ("iMac19,1", Tahoe, false),
            ("iMac19,1", Sequoia, true),
            ("iMac19,1", Mojave, true),
            ("iMac19,1", HighSierra, false),
            ("iMacPro1,1", Tahoe, false),
            ("iMacPro1,1", Sequoia, true),
            ("iMacPro1,1", HighSierra, true),
            ("MacPro7,1", Tahoe, true),
            ("MacPro7,1", Mojave, false),
            ("MacPro6,1", Monterey, true),
            ("MacPro6,1", Ventura, false),
            ("MacPro5,1", Mojave, true),
            ("MacPro5,1", Catalina, false),
            ("iMac18,3", Ventura, true),
            ("iMac18,3", Sonoma, false),
            ("MacBookPro14,1", Ventura, true),
            ("MacBookPro14,1", Sonoma, false),
            ("MacBookAir8,1", Sonoma, true),
            ("MacBookAir8,1", Sequoia, false),
            ("MacBookAir8,1", HighSierra, false),
            ("MacBookAir9,1", Sequoia, true),
            ("MacBookAir9,1", Tahoe, false),
            ("MacBookPro16,2", Tahoe, true),
            ("MacBookPro16,3", Tahoe, false),
            ("MacBookPro15,2", HighSierra, true),
            ("MacBookPro15,3", HighSierra, false),
            ("Macmini8,1", Mojave, true),
            ("Macmini8,1", Tahoe, false),
            ("iMac17,1", Monterey, true),
            ("iMac17,1", Ventura, false),
            ("iMac14,4", BigSur, true),
            ("iMac14,4", Monterey, false),
            ("iMac13,2", Catalina, true),
            ("iMac13,2", BigSur, false),
            ("iMac10,1", HighSierra, true),
            ("iMac10,1", Mojave, false),
            ("MacBook8,1", BigSur, true),
            ("MacBook8,1", Monterey, false),
            ("macbookpro16,1", Tahoe, true),
            ("iMac9,1", HighSierra, false),
            ("Nonsense1,1", HighSierra, false),
        ];
        for (model, version, want) in matrix {
            assert_eq!(supports(model, *version), *want, "{model} on {version:?}");
        }
    }

    #[test]
    fn secure_boot_models() {
        let t2: &[(&str, &str)] = &[
            ("iMacPro1,1", "j137"),
            ("MacBookPro15,1", "j680"),
            ("MacBookPro15,2", "j132"),
            ("Macmini8,1", "j174"),
            ("MacBookAir8,1", "j140k"),
            ("MacBookPro15,3", "j780"),
            ("MacBookPro15,4", "j213"),
            ("MacBookAir8,2", "j140a"),
            ("MacBookPro16,1", "j152f"),
            ("MacPro7,1", "j160"),
            ("MacBookAir9,1", "j230k"),
            ("MacBookPro16,2", "j214k"),
            ("MacBookPro16,3", "j223"),
            ("MacBookPro16,4", "j215"),
            ("iMac20,1", "j185"),
            ("iMac20,2", "j185f"),
        ];
        for (model, sbm) in t2 {
            assert_eq!(
                find(model).and_then(|m| m.secure_boot_model),
                Some(*sbm),
                "{model}"
            );
        }
        let t2_count = all()
            .iter()
            .filter(|m| m.secure_boot_model.is_some())
            .count();
        assert_eq!(t2_count, t2.len());
        assert_eq!(find("iMac19,1").and_then(|m| m.secure_boot_model), None);
    }

    #[test]
    fn board_id_lookup() {
        assert_eq!(
            find_by_board_id("Mac-CFF7D910A743CAAF").map(|m| m.model),
            Some("iMac20,1")
        );
        assert_eq!(
            find_by_board_id("mac-27ad2f918ae68f61").map(|m| m.model),
            Some("MacPro7,1")
        );
        assert_eq!(
            find_by_board_id("Mac-65CE76090165799A").map(|m| m.model),
            Some("iMac17,1")
        );
        assert_eq!(
            find_by_board_id("Mac-F221DCC8").map(|m| m.model),
            Some("iMac10,1")
        );
        assert!(find_by_board_id("Mac-0000000000000000").is_none());
        assert!(find_by_board_id("").is_none());
        // MacPro4,1 and MacPro5,1 share Mac-F221BEC8; only MacPro5,1 is listed.
        assert_eq!(
            find_by_board_id("Mac-F221BEC8").map(|m| m.model),
            Some("MacPro5,1")
        );
        for m in all() {
            assert_eq!(
                find_by_board_id(m.board_id).map(|f| f.model),
                Some(m.model),
                "board-id of {} is not unique",
                m.model
            );
        }
    }

    #[test]
    fn board_ids_and_kinds() {
        let ids: &[(&str, &str)] = &[
            ("iMac19,1", "Mac-AA95B1DDAB278B95"),
            ("iMac20,1", "Mac-CFF7D910A743CAAF"),
            ("iMac20,2", "Mac-AF89B6D9451A490B"),
            ("iMacPro1,1", "Mac-7BA5B2D9E42DDD94"),
            ("MacPro7,1", "Mac-27AD2F918AE68F61"),
            ("MacPro6,1", "Mac-F60DEB81FF30ACF6"),
            ("MacBookPro16,2", "Mac-5F9802EFE386AA28"),
            ("iMac17,1", "Mac-DB15BD556843C820"),
            ("iMac10,1", "Mac-F2268CC8"),
        ];
        for (model, id) in ids {
            assert_eq!(find(model).map(|m| m.board_id), Some(*id), "{model}");
        }
        assert_eq!(find("MacPro7,1").map(|m| m.kind), Some(Workstation));
        assert_eq!(find("iMacPro1,1").map(|m| m.kind), Some(Workstation));
        assert_eq!(find("Macmini8,1").map(|m| m.kind), Some(Desktop));
        assert_eq!(find("MacBookAir9,1").map(|m| m.kind), Some(Laptop));
        for m in all() {
            let laptop = m.model.starts_with("MacBook");
            assert_eq!(m.kind == Laptop, laptop, "{}", m.model);
        }
    }
}
