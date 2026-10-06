//! PlatformInfo identity generation: serial + MLB valid for the chosen model
//! (macserial format and checksum rules), SystemUUID, ROM.
//!
//! The native generator and its model table are a port of `macserial`
//! (acidanthera OpenCorePkg `Utilities/macserial`, BSD-3-Clause,
//! Copyright (c) 2018-2020 vit9696, Copyright (c) 2020 Matis Schotte).

use std::io::Read;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use rand::Rng;

use crate::domain::model::{hex_upper, PlatformIdentity};
use crate::error::AppError;

/// Apple base 34: digits and capitals without `I` and `O`.
const BASE34: &[u8; 34] = b"0123456789ABCDEFGHJKLMNPQRSTUVWXYZ";
/// 12-character serial year symbols: two per year (first / second half).
const SERIAL_YEAR: &[u8; 20] = b"CDFGHJKLMNPQRSTVWXYZ";
/// 12-character serial week symbols, indexed by week 1..=53.
const SERIAL_WEEK: &[u8; 54] = b"0123456789CDFGHJKLMNPQRTVWX123456789CDFGHJKLMNPQRTVWXY";
/// Week symbols as read back when deriving the MLB (macserial keeps the
/// original reverse-engineered table, including `S`/`Y`/`Z`).
const MLB_WEEK: &[u8; 29] = b"123456789CDFGHJKLMNPQRSTVWXYZ";
/// Second-half year symbols (add 26 weeks).
const SECOND_HALF_YEAR: &[u8; 10] = b"DGJLNQSVXZ";

const MLB_BLOCK1: [&str; 28] = [
    "200", "600", "403", "404", "405", "303", "108", "207", "609", "501", "306", "102", "701",
    "301", "501", "101", "300", "130", "100", "270", "310", "902", "104", "401", "902", "500",
    "700", "802",
];
const MLB_BLOCK2: [&str; 7] = ["GU", "4N", "J9", "QX", "OP", "CD", "GU"];
const MLB_BLOCK3: [&str; 11] = [
    "1H", "1M", "AD", "1F", "A8", "UE", "JA", "JC", "8C", "CB", "FB",
];

const SERIAL_LINE_REPR_MAX: u32 = 1155;
const SERIAL_LINE_MAX: u32 = 3399;
const MLB_ATTEMPTS: usize = 100_000;
const MACSERIAL_TIMEOUT: Duration = Duration::from_secs(15);
const MACSERIAL_OUTPUT_LIMIT: u64 = 64 * 1024;

/// macserial data for one model: location prefix of its base serial, the
/// model code and board code macserial picks (always the first listed), the
/// production years and the preferred year (0 = random production year).
struct ModelInfo {
    model: &'static str,
    country: &'static str,
    model_code: &'static str,
    board_code: &'static str,
    years: &'static [u16],
    preferred_year: u16,
}

const fn m(
    model: &'static str,
    country: &'static str,
    model_code: &'static str,
    board_code: &'static str,
    years: &'static [u16],
    preferred_year: u16,
) -> ModelInfo {
    ModelInfo {
        model,
        country,
        model_code,
        board_code,
        years,
        preferred_year,
    }
}

/// Every model from macserial 2.1.8 (OpenCore 1.0.8 `modelinfo_autogen.h`)
/// that can run macOS 10.13 or newer.
#[rustfmt::skip]
static MODELS: &[ModelInfo] = &[
    m("MacBook6,1", "45", "GAY", "000", &[2009, 2010], 0),
    m("MacBook7,1", "45", "F5X", "000", &[2010, 2011], 0),
    m("MacBook8,1", "C02", "GCN3", "FV36", &[2015, 2016], 0),
    m("MacBook9,1", "C02", "HDNK", "FV48", &[2016, 2017], 0),
    m("MacBook10,1", "C02", "HH27", "HJ9L", &[2017, 2018, 2019], 0),
    m("MacBookAir3,1", "C02", "DDQW", "DF83", &[2010, 2011], 0),
    m("MacBookAir3,2", "C02", "DDR3", "DCWQ", &[2010, 2011], 0),
    m("MacBookAir4,1", "C02", "DJY8", "DK9L", &[2011, 2012], 0),
    m("MacBookAir4,2", "C02", "DJWT", "DP1G", &[2011, 2012], 0),
    m("MacBookAir5,1", "C02", "DRV6", "DYKF", &[2012, 2013], 0),
    m("MacBookAir5,2", "C02", "DRVC", "F25Q", &[2012, 2013], 0),
    m("MacBookAir6,1", "C02", "F5N7", "FD09", &[2013, 2014, 2015], 0),
    m("MacBookAir6,2", "C02", "F5V7", "FD47", &[2013, 2014, 2015], 0),
    m("MacBookAir7,1", "C02", "GFWK", "G90F", &[2015, 2016], 0),
    m("MacBookAir7,2", "C02", "G940", "G91Q", &[2015, 2016, 2017, 2018, 2019], 0),
    m("MacBookAir8,1", "C02", "JK78", "KN2R", &[2018, 2019], 0),
    m("MacBookAir8,2", "FVF", "LYWM", "0000", &[2019, 2020], 0),
    m("MacBookAir9,1", "FVF", "MNHP", "0000", &[2020], 0),
    m("MacBookPro6,1", "C02", "DC79", "DCMV", &[2010, 2011], 0),
    m("MacBookPro6,2", "CK", "AGW", "FYR", &[2010, 2011], 0),
    m("MacBookPro7,1", "CK", "ATM", "000", &[2010, 2011], 0),
    m("MacBookPro8,1", "W89", "DH2G", "DM6D", &[2011, 2012], 0),
    m("MacBookPro8,2", "C02", "DF8X", "DMMN", &[2011, 2012], 0),
    m("MacBookPro8,3", "W88", "DF93", "DM5K", &[2011, 2012], 0),
    m("MacBookPro9,1", "C02", "F1G4", "F327", &[2012, 2013], 0),
    m("MacBookPro9,2", "C02", "DTY3", "F1YJ", &[2012, 2013, 2014, 2015, 2016], 0),
    m("MacBookPro10,1", "C02", "DKQ1", "DY3V", &[2012, 2013], 0),
    m("MacBookPro10,2", "C02", "DR53", "F16P", &[2012, 2013], 0),
    m("MacBookPro11,1", "C02", "FH00", "FH31", &[2013, 2014, 2015], 0),
    m("MacBookPro11,2", "C02", "G86R", "FJQW", &[2013, 2014, 2015], 0),
    m("MacBookPro11,3", "C02", "FR1M", "FP52", &[2013, 2014, 2015], 0),
    m("MacBookPro11,4", "C02", "G8WN", "GDQP", &[2015, 2016, 2017, 2018], 0),
    m("MacBookPro11,5", "C02", "G85Y", "GF2C", &[2015, 2016, 2017, 2018], 0),
    m("MacBookPro12,1", "C02", "H1DP", "GDVV", &[2015, 2016, 2017], 0),
    m("MacBookPro13,1", "C17", "GVC1", "HMHK", &[2016, 2017], 0),
    m("MacBookPro13,2", "C02", "GYFH", "H9W8", &[2016, 2017], 0),
    m("MacBookPro13,3", "C02", "GTFN", "HCF9", &[2016, 2017], 0),
    m("MacBookPro14,1", "C02", "HV29", "HWVP", &[2017, 2018, 2019], 0),
    m("MacBookPro14,2", "C02", "HV2N", "HRPC", &[2017, 2018], 0),
    m("MacBookPro14,3", "C02", "HTD5", "J1JH", &[2017, 2018], 0),
    m("MacBookPro15,1", "C02", "KGYG", "JP4F", &[2018, 2019], 0),
    m("MacBookPro15,2", "C02", "JHCC", "JH4R", &[2018, 2019, 2020], 0),
    m("MacBookPro15,3", "C02", "LVCG", "0000", &[2019], 0),
    m("MacBookPro15,4", "FVF", "L40Y", "0000", &[2019, 2020], 0),
    m("MacBookPro16,1", "C02", "MD6N", "N9PR", &[2019, 2020, 2021], 0),
    m("MacBookPro16,2", "C02", "ML7H", "P8PG", &[2020, 2021], 0),
    m("MacBookPro16,3", "C02", "P3XY", "0000", &[2020], 0),
    m("MacBookPro16,4", "C02", "MD6T", "0000", &[2020, 2021], 0),
    m("MacPro5,1", "CK", "EUH", "BH8", &[2010, 2011, 2012, 2013], 2011),
    m("MacPro6,1", "F5K", "F9VM", "FHDD", &[2013, 2014, 2015, 2016, 2017, 2018, 2019], 0),
    m("MacPro7,1", "F5K", "P7QM", "K3F7", &[2019, 2020, 2021, 2022, 2023], 0),
    m("Macmini4,1", "C02", "DD6H", "DC2D", &[2010, 2011], 0),
    m("Macmini5,1", "C07", "DJD0", "DKP2", &[2011, 2012], 0),
    m("Macmini5,2", "C07", "DJD1", "DK22", &[2011, 2012], 0),
    m("Macmini5,3", "C07", "DKDJ", "DHDN", &[2011, 2012], 0),
    m("Macmini6,1", "C07", "DY3H", "F1HC", &[2012, 2013, 2014], 0),
    m("Macmini6,2", "C07", "DWYN", "DVF9", &[2012, 2013, 2014], 0),
    m("Macmini7,1", "C02", "G1J0", "G0MC", &[2014, 2015, 2016, 2017, 2018], 0),
    m("Macmini8,1", "C07", "JYVX", "KXPG", &[2018, 2019, 2020, 2021, 2022, 2023], 0),
    m("iMac10,1", "W8", "5PE", "000", &[2009, 2010], 0),
    m("iMac11,1", "G8", "5PJ", "000", &[2009, 2010], 0),
    m("iMac11,2", "W8", "DB7", "DCJN", &[2010, 2011], 0),
    m("iMac11,3", "QP", "DNR", "000", &[2010, 2011], 0),
    m("iMac12,1", "W80", "DHJF", "DJWK", &[2011, 2012, 2013], 0),
    m("iMac12,2", "W88", "DHJQ", "DJWM", &[2011, 2012], 0),
    m("iMac13,1", "C02", "DNCT", "DYWF", &[2012, 2013, 2014], 0),
    m("iMac13,2", "C02", "DNCW", "F2FR", &[2012, 2013], 0),
    m("iMac13,3", "C02", "FFYW", "F8GR", &[2013], 0),
    m("iMac14,1", "D25", "FWJH", "FM59", &[2013, 2014, 2015], 0),
    m("iMac14,2", "D25", "F8JC", "F8YL", &[2013, 2014, 2015], 0),
    m("iMac14,3", "D25", "F8J3", "F9RR", &[2013, 2014, 2015], 0),
    m("iMac14,4", "D25", "FY0T", "G36D", &[2014, 2015], 0),
    m("iMac15,1", "C02", "FY10", "G2Y7", &[2014, 2015], 0),
    m("iMac16,1", "C02", "GF1J", "GH34", &[2015, 2016, 2017], 0),
    m("iMac16,2", "DGK", "GG7F", "GQRY", &[2015, 2016, 2017], 0),
    m("iMac17,1", "C02", "GG7L", "GPF7", &[2015, 2016, 2017], 0),
    m("iMac18,1", "C02", "H7JY", "H69F", &[2017, 2018, 2019, 2020, 2021], 0),
    m("iMac18,2", "C02", "J1G5", "J0DX", &[2017, 2018, 2019], 0),
    m("iMac18,3", "C02", "J1GJ", "J0PG", &[2017, 2018, 2019], 0),
    m("iMac19,1", "C02", "JV3Q", "LNV9", &[2019, 2020], 0),
    m("iMac19,2", "C02", "JWDW", "KGQG", &[2019, 2020, 2021], 0),
    m("iMac20,1", "C02", "PN5T", "PHC1", &[2020, 2021, 2022], 0),
    m("iMac20,2", "C02", "046M", "0000", &[2020, 2021, 2022], 0),
    m("iMacPro1,1", "C02", "HX87", "JG36", &[2017, 2018, 2019, 2020, 2021], 0),
];

/// Apple OUIs used as the first three ROM bytes when no NIC MAC is known
/// (same list as corpnewt/GenSMBIOS `Scripts/prefix.json`, MIT).
static APPLE_OUIS: &[u32] = &[
    0x000393, 0x000A27, 0x000A95, 0x000D93, 0x0010FA, 0x001124, 0x001451, 0x0016CB, 0x0017F2,
    0x0019E3, 0x001B63, 0x001CB3, 0x001D4F, 0x001E52, 0x001EC2, 0x001F5B, 0x001FF3, 0x0021E9,
    0x002241, 0x002312, 0x002332, 0x00236C, 0x0023DF, 0x002436, 0x002500, 0x00254B, 0x0025BC,
    0x002608, 0x00264A, 0x0026B0, 0x0026BB, 0x003065, 0x003EE1, 0x0050E4, 0x0056CD, 0x006171,
    0x006D52, 0x008865, 0x00B362, 0x00C610, 0x00CDFE, 0x00F4B9, 0x00F76F, 0x040CCE, 0x041552,
    0x041E64, 0x042665, 0x04489A, 0x044BED, 0x0452F3, 0x045453, 0x0469F8, 0x04D3CF, 0x04DB56,
    0x04E536, 0x04F13E, 0x04F7E4, 0x086698, 0x086D41, 0x087045, 0x087402, 0x0C1539, 0x0C3021,
    0x0C3E9F, 0x0C4DE9, 0x0C5101, 0x0C74C2, 0x0C771A, 0x0CBC9F, 0x0CD746, 0x101C0C, 0x1040F3,
    0x10417F, 0x1093E9, 0x109ADD, 0x10DDB1, 0x14109F, 0x145A05, 0x148FC6, 0x1499E2, 0x14BD61,
    0x182032, 0x183451, 0x186590, 0x189EFC, 0x18AF61, 0x18AF8F, 0x18E7F4, 0x18EE69, 0x18F643,
    0x1C1AC0, 0x1C5CF2, 0x1C9148, 0x1C9E46, 0x1CABA7, 0x1CE62B, 0x203CAE, 0x20768F, 0x2078F0,
    0x207D74, 0x209BCD, 0x20A2E4, 0x20AB37, 0x20C9D0, 0x241EEB, 0x24240E, 0x245BA7, 0x24A074,
    0x24A2E1, 0x24AB81, 0x24E314, 0x24F094, 0x280B5C, 0x283737, 0x285AEB, 0x286AB8, 0x286ABA,
    0x28A02B, 0x28CFDA, 0x28CFE9, 0x28E02C, 0x28E14C, 0x28E7CF, 0x28ED6A, 0x28F076, 0x2C1F23,
    0x2C200B, 0x2C3361, 0x2CB43A, 0x2CBE08, 0x2CF0A2, 0x2CF0EE, 0x3010E4, 0x30636B, 0x3090AB,
    0x30F7C5, 0x341298, 0x34159E, 0x34363B, 0x3451C9, 0x34A395, 0x34AB37, 0x34C059, 0x34E2FD,
    0x380F4A, 0x38484C, 0x3871DE, 0x38B54D, 0x38C986, 0x38CADA, 0x3C0754, 0x3C15C2, 0x3CAB8E,
    0x3CD0F8, 0x3CE072, 0x403004, 0x40331A, 0x403CFC, 0x404D7F, 0x406C8F, 0x40A6D9, 0x40B395,
    0x40D32D, 0x440010, 0x442A60, 0x444C0C, 0x44D884, 0x44FB42, 0x483B38, 0x48437C, 0x484BAA,
    0x4860BC, 0x48746E, 0x48A195, 0x48BF6B, 0x48D705, 0x48E9F1, 0x4C3275, 0x4C57CA, 0x4C74BF,
    0x4C7C5F, 0x4C8D79, 0x4CB199, 0x503237, 0x507A55, 0x5082D5, 0x50EAD6, 0x542696, 0x544E90,
    0x54724F, 0x549F13, 0x54AE27, 0x54E43A, 0x54EAA8, 0x581FAA, 0x58404E, 0x5855CA, 0x587F57,
    0x58B035, 0x5C5948, 0x5C8D4E, 0x5C95AE, 0x5C969D, 0x5C97F3, 0x5CADCF, 0x5CF5DA, 0x5CF7E6,
    0x5CF938, 0x600308, 0x60334B, 0x606944, 0x609217, 0x609AC1, 0x60A37D, 0x60C547, 0x60D9C7,
    0x60F445, 0x60F81D, 0x60FACD, 0x60FB42, 0x60FEC5, 0x64200C, 0x6476BA, 0x649ABE, 0x64A3CB,
    0x64A5C3, 0x64B0A6, 0x64B9E8, 0x64E682, 0x680927, 0x685B35, 0x68644B, 0x68967B, 0x689C70,
    0x68A86D, 0x68AE20, 0x68D93C, 0x68DBCA, 0x68FB7E, 0x6C19C0, 0x6C3E6D, 0x6C4008, 0x6C709F,
    0x6C72E7, 0x6C8DC1, 0x6C94F8, 0x6CAB31, 0x6CC26B, 0x701124, 0x7014A6, 0x703EAC, 0x70480F,
    0x705681, 0x70700D, 0x7073CB, 0x7081EB, 0x70A2B3, 0x70CD60, 0x70DEE2, 0x70E72C, 0x70ECE4,
    0x70F087, 0x741BB2, 0x748114, 0x748D08, 0x74E1B6, 0x74E2F5, 0x7831C1, 0x783A84, 0x784F43,
    0x786C1C, 0x787E61, 0x789F70, 0x78A3E4, 0x78CA39, 0x78D75F, 0x78FD94, 0x7C0191, 0x7C04D0,
    0x7C11BE, 0x7C5049, 0x7C6D62, 0x7C6DF8, 0x7CC3A1, 0x7CC537, 0x7CD1C3, 0x7CF05F, 0x7CFADF,
    0x80006E, 0x804971, 0x80929F, 0x80BE05, 0x80D605, 0x80E650, 0x80EA96, 0x80ED2C, 0x842999,
    0x843835, 0x84788B, 0x848506, 0x8489AD, 0x848E0C, 0x84A134, 0x84B153, 0x84FCAC, 0x84FCFE,
    0x881FA1, 0x885395, 0x8863DF, 0x8866A5, 0x886B6E, 0x88C663, 0x88CB87, 0x88E87F, 0x8C006D,
    0x8C2937, 0x8C2DAA, 0x8C5877, 0x8C7B9D, 0x8C7C92, 0x8C8EF2, 0x8C8FE9, 0x8CFABA, 0x9027E4,
    0x903C92, 0x9060F1, 0x907240, 0x90840D, 0x908D6C, 0x90B0ED, 0x90B21F, 0x90B931, 0x90C1C6,
    0x90FD61, 0x949426, 0x94E96A, 0x94F6A3, 0x9801A7, 0x9803D8, 0x9810E8, 0x985AEB, 0x989E63,
    0x98B8E3, 0x98D6BB, 0x98E0D9, 0x98F0AB, 0x98FE94, 0x9C04EB, 0x9C207B, 0x9C293F, 0x9C35EB,
    0x9C4FDA, 0x9C84BF, 0x9C8BA0, 0x9CF387, 0x9CF48E, 0x9CFC01, 0xA01828, 0xA03BE3, 0xA0999B,
    0xA0D795, 0xA0EDCD, 0xA43135, 0xA45E60, 0xA46706, 0xA4B197, 0xA4B805, 0xA4C361, 0xA4D18C,
    0xA4D1D2, 0xA4F1E8, 0xA82066, 0xA85B78, 0xA860B6, 0xA8667F, 0xA886DD, 0xA88808, 0xA88E24,
    0xA8968A, 0xA8BBCF, 0xA8FAD8, 0xAC293A, 0xAC3C0B, 0xAC61EA, 0xAC7F3E, 0xAC87A3, 0xACBC32,
    0xACCF5C, 0xACFDEC, 0xB03495, 0xB0481A, 0xB065BD, 0xB0702D, 0xB09FBA, 0xB418D1, 0xB44BD2,
    0xB48B19, 0xB49CDF, 0xB4F0AB, 0xB8098A, 0xB817C2, 0xB844D9, 0xB853AC, 0xB8782E, 0xB88D12,
    0xB8C75D, 0xB8E856, 0xB8F6B1, 0xB8FF61, 0xBC3BAF, 0xBC4CC4, 0xBC52B7, 0xBC5436, 0xBC6778,
    0xBC6C21, 0xBC926B, 0xBC9FEF, 0xBCA920, 0xBCEC5D, 0xC01ADA, 0xC06394, 0xC0847A, 0xC09F42,
    0xC0CCF8, 0xC0CECD, 0xC0D012, 0xC0F2FB, 0xC42C03, 0xC4B301, 0xC81EE7, 0xC82A14, 0xC8334B,
    0xC869CD, 0xC86F1D, 0xC88550, 0xC8B5B7, 0xC8BCC8, 0xC8E0EB, 0xC8F650, 0xCC088D, 0xCC08E0,
    0xCC20E8, 0xCC25EF, 0xCC29F5, 0xCC4463, 0xCC785F, 0xCCC760, 0xD0034B, 0xD023DB, 0xD02598,
    0xD03311, 0xD04F7E, 0xD0A637, 0xD0C5F3, 0xD0E140, 0xD4619D, 0xD49A20, 0xD4DCCD, 0xD4F46F,
    0xD8004D, 0xD81D72, 0xD83062, 0xD89695, 0xD89E3F, 0xD8A25E, 0xD8BB2C, 0xD8CF9C, 0xD8D1CB,
    0xDC0C5C, 0xDC2B2A, 0xDC2B61, 0xDC3714, 0xDC415F, 0xDC86D8, 0xDC9B9C, 0xDCA4CA, 0xDCA904,
    0xE05F45, 0xE06678, 0xE0ACCB, 0xE0B52D, 0xE0B9BA, 0xE0C767, 0xE0C97A, 0xE0F5C6, 0xE0F847,
    0xE425E7, 0xE48B7F, 0xE498D6, 0xE49A79, 0xE4C63D, 0xE4CE8F, 0xE4E4AB, 0xE8040B, 0xE80688,
    0xE8802E, 0xE88D28, 0xE8B2AC, 0xEC3586, 0xEC852F, 0xECADB8, 0xF02475, 0xF07960, 0xF099BF,
    0xF0B0E7, 0xF0B479, 0xF0C1F1, 0xF0CBA1, 0xF0D1A9, 0xF0DBE2, 0xF0DBF8, 0xF0DCE2, 0xF0F61C,
    0xF40F24, 0xF41BA1, 0xF431C3, 0xF437B7, 0xF45C89, 0xF4F15A, 0xF4F951, 0xF80377, 0xF81EDF,
    0xF82793, 0xF86214, 0xFC253F, 0xFCD848, 0xFCE998, 0xFCFC48,
];

/// Generate a fresh identity for `model`. Uses the `macserial` binary from the
/// OpenCore package when it can run on this host, otherwise the native
/// generator. ROM = primary NIC MAC when given, else random bytes with an
/// Apple OUI.
///
/// Blocks for up to 15 s while macserial runs; call it from a blocking task.
pub fn generate_identity(
    model: &str,
    macserial: Option<&Path>,
    mac_address: Option<&str>,
) -> Result<PlatformIdentity, AppError> {
    if !model_name_valid(model) {
        return Err(AppError::new(
            "SMBIOS_MODEL_INVALID",
            format!("'{model}' is not a Mac model identifier"),
        ));
    }

    // A serial for another model (e.g. from a macserial with a different
    // table) is as useless as no serial at all.
    let model_code = model_info(model).map(|info| info.model_code);
    let from_tool = macserial.and_then(|path| match run_macserial(path, model) {
        Ok(output) => {
            let parsed = parse_macserial_output(&output)
                .filter(|(serial, _)| model_code.is_none_or(|code| serial.ends_with(code)));
            if parsed.is_none() {
                // The output carries a serial number, so it is not logged.
                tracing::warn!(model, "unexpected macserial output, using the built-in generator");
            }
            parsed
        }
        Err(err) => {
            tracing::warn!(model, path = %path.display(), %err, "macserial failed, using the built-in generator");
            None
        }
    });

    let (serial, mlb) = from_tool
        .or_else(|| native_serial_and_mlb(model))
        .ok_or_else(|| {
            AppError::new(
                "SMBIOS_MODEL_UNKNOWN",
                format!("Cannot generate a serial number for {model}"),
            )
            .with_suggestion(
                "Pick another SMBIOS model or generate the identity with macserial/GenSMBIOS.",
            )
        })?;

    let rom = match mac_address.and_then(rom_from_mac) {
        Some(rom) => rom,
        None => {
            if mac_address.is_some() {
                tracing::warn!("MAC address is not usable as ROM, using a random Apple ROM");
            }
            random_rom()
        }
    };

    Ok(PlatformIdentity {
        model: model.to_string(),
        serial,
        mlb,
        system_uuid: uuid::Uuid::new_v4().hyphenated().to_string().to_uppercase(),
        rom,
    })
}

/// Native serial/MLB generator (port of macserial's rules for the models in
/// `smbios_db`). Returns (serial, mlb).
pub fn native_serial_and_mlb(model: &str) -> Option<(String, String)> {
    let info = model_info(model)?;
    let mut rng = rand::rng();
    let serial = native_serial(info, &mut rng)?;
    let mlb = native_mlb(info, &serial, &mut rng)?;
    Some((serial, mlb))
}

/// 17-character MLB base-34 checksum validation (macserial `verify_mlb_checksum`).
/// Legacy 13-character MLBs produced by macserial satisfy the same checksum
/// and are accepted too; every other length is rejected.
pub fn mlb_checksum_valid(mlb: &str) -> bool {
    matches!(mlb.len(), 13 | 17) && checksum_ok(mlb)
}

/// Structural check of a generated serial: 11 (legacy) or 12 characters of
/// Apple base 34; 12-character serials must carry valid year and week symbols.
pub fn serial_format_valid(serial: &str) -> bool {
    let bytes = serial.as_bytes();
    if !bytes.iter().all(|b| BASE34.contains(b)) {
        return false;
    }
    match bytes.len() {
        11 => bytes[2].is_ascii_digit() && bytes[3].is_ascii_digit() && bytes[4].is_ascii_digit(),
        12 => SERIAL_YEAR.contains(&bytes[3]) && SERIAL_WEEK[1..].contains(&bytes[4]),
        _ => false,
    }
}

/// Parse `macserial -m <model> -n 1` output ("SERIAL | MLB"). Returns the
/// first line whose serial and MLB pass the format and checksum checks.
pub fn parse_macserial_output(output: &str) -> Option<(String, String)> {
    output.lines().find_map(|line| {
        let mut parts = line.split('|').map(str::trim);
        let serial = parts.next()?;
        let mlb = parts.next()?;
        if parts.next().is_some() {
            return None;
        }
        (serial_format_valid(serial) && mlb_checksum_valid(mlb))
            .then(|| (serial.to_string(), mlb.to_string()))
    })
}

/// ROM value (12 upper-case hex digits) from a MAC address in any common
/// notation ("AA:BB:CC:DD:EE:FF", "aa-bb-cc-dd-ee-ff", "aabb.ccdd.eeff").
/// Rejects all-zero, broadcast and multicast addresses.
pub fn rom_from_mac(mac: &str) -> Option<String> {
    let hex: String = mac
        .chars()
        .filter(|c| !matches!(c, ':' | '-' | '.' | ' '))
        .collect();
    if hex.len() != 12 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let hex = hex.to_ascii_uppercase();
    let first = u8::from_str_radix(&hex[..2], 16).ok()?;
    if hex.chars().all(|c| c == '0') || hex.chars().all(|c| c == 'F') || first & 1 == 1 {
        return None;
    }
    Some(hex)
}

/// Random ROM: an Apple OUI followed by three random bytes.
pub fn random_rom() -> String {
    let mut rng = rand::rng();
    let oui = APPLE_OUIS[rng.random_range(0..APPLE_OUIS.len())];
    let tail: [u8; 3] = rng.random();
    let bytes = [
        (oui >> 16) as u8,
        (oui >> 8) as u8,
        oui as u8,
        tail[0],
        tail[1],
        tail[2],
    ];
    hex_upper(&bytes)
}

fn model_info(model: &str) -> Option<&'static ModelInfo> {
    MODELS.iter().find(|m| m.model.eq_ignore_ascii_case(model))
}

/// "iMac20,1", "MacBookPro16,2", ...: letters, then "<major>,<minor>".
fn model_name_valid(model: &str) -> bool {
    let Some(split) = model.find(|c: char| c.is_ascii_digit()) else {
        return false;
    };
    let (name, version) = model.split_at(split);
    let Some((major, minor)) = version.split_once(',') else {
        return false;
    };
    !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphabetic())
        && !major.is_empty()
        && major.chars().all(|c| c.is_ascii_digit())
        && !minor.is_empty()
        && minor.chars().all(|c| c.is_ascii_digit())
}

fn base34_value(c: u8) -> Option<usize> {
    BASE34.iter().position(|&b| b == c)
}

/// macserial `verify_mlb_checksum`: weight 3 on positions with the same
/// parity as the length, weight 1 elsewhere; sum must be divisible by 34.
fn checksum_ok(mlb: &str) -> bool {
    let len = mlb.len();
    let mut sum = 0usize;
    for (i, c) in mlb.bytes().enumerate() {
        let Some(value) = base34_value(c) else {
            return false;
        };
        let weight = if (i & 1) == (len & 1) { 3 } else { 1 };
        sum += weight * value;
    }
    len > 0 && sum.is_multiple_of(34)
}

/// macserial `get_serial` for a known model with no user-supplied fields.
fn native_serial(info: &ModelInfo, rng: &mut impl Rng) -> Option<String> {
    let year = if info.preferred_year > 0 {
        u32::from(info.preferred_year)
    } else {
        u32::from(
            *info
                .years
                .get(rng.random_range(0..info.years.len().max(1)))?,
        )
    };
    // Week 53 is too rare to bother with.
    let week = rng.random_range(1..=52u32);
    let line = rng.random_range(0..=SERIAL_LINE_MAX);

    let mut serial = String::with_capacity(12);
    serial.push_str(info.country);
    if info.country.len() == 2 {
        if !(2003..=2012).contains(&year) {
            return None;
        }
        serial.push(char::from(b'0' + ((year - 2000) % 10) as u8));
        serial.push_str(&format!("{week:02}"));
    } else {
        if !(2010..=2030).contains(&year) {
            return None;
        }
        let base = if year >= 2020 { 2020 } else { 2010 };
        let index = ((year - base) * 2 + u32::from(week >= 27)) as usize;
        serial.push(char::from(*SERIAL_YEAR.get(index)?));
        serial.push(char::from(SERIAL_WEEK[week as usize]));
    }

    // Production line: base34[S1] * 68 + base34[S2] * 34 + base34[S3].
    let rmin = if line > SERIAL_LINE_REPR_MAX {
        (line - SERIAL_LINE_REPR_MAX).div_ceil(68)
    } else {
        0
    };
    let rest = line - rmin * 68;
    for value in [rmin, rest / 34, rest % 34] {
        serial.push(char::from(BASE34[value as usize]));
    }
    serial.push_str(info.model_code);
    Some(serial)
}

/// macserial `get_mlb`: derives year/week from the serial and retries random
/// blocks until the checksum holds.
fn native_mlb(info: &ModelInfo, serial: &str, rng: &mut impl Rng) -> Option<String> {
    let bytes = serial.as_bytes();
    let legacy = info.country.len() == 2;
    let (mut year, mut week) = if legacy {
        let digit = |i: usize| {
            bytes
                .get(i)
                .filter(|b| b.is_ascii_digit())
                .map(|b| u32::from(b - b'0'))
        };
        (digit(2)?, digit(3)? * 10 + digit(4)?)
    } else {
        let year_symbol = *bytes.get(3)?;
        let week_symbol = *bytes.get(4)?;
        let year = SERIAL_YEAR
            .iter()
            .position(|&b| b == year_symbol)
            .map_or(0, |i| (i / 2) as u32);
        let mut week = if SECOND_HALF_YEAR.contains(&year_symbol) {
            27
        } else {
            0
        };
        if let Some(i) = MLB_WEEK.iter().position(|&b| b == week_symbol) {
            week += i as u32 + 1;
        }
        if week < 1 {
            return None;
        }
        (year, week)
    };

    week -= 1;
    if week == 0 {
        week = 53;
        year = if year == 0 { 9 } else { year - 1 };
    }

    for _ in 0..MLB_ATTEMPTS {
        let mlb = if legacy {
            let code = legacy_mlb_code(rng);
            let suffix = char::from(BASE34[rng.random_range(0..BASE34.len())]);
            format!(
                "{}{year}{week:02}0{code}{}{suffix}",
                info.country, info.board_code
            )
        } else {
            let part1 = MLB_BLOCK1[rng.random_range(0..MLB_BLOCK1.len())];
            let part2 = MLB_BLOCK2[rng.random_range(0..MLB_BLOCK2.len())];
            let part3 = MLB_BLOCK3[rng.random_range(0..MLB_BLOCK3.len())];
            format!(
                "{}{year}{week:02}{part1}{part2}{}{part3}",
                info.country, info.board_code
            )
        };
        if checksum_ok(&mlb) {
            return Some(mlb);
        }
    }
    None
}

/// Three base-34 symbols for legacy MLBs (macserial `get_ascii7` "CCC" code).
fn legacy_mlb_code(rng: &mut impl Rng) -> String {
    loop {
        let value = rng.random_range(0..=0x7FFEu32).wrapping_mul(0x0073_BA1C);
        if let Some(code) = ascii7(value) {
            return code;
        }
    }
}

fn ascii7(mut value: u32) -> Option<String> {
    if value < 1_000_000 {
        return None;
    }
    while value > 10_000_000 {
        value /= 10;
    }
    let mut digits = Vec::with_capacity(6);
    while value > 0 {
        digits.push(BASE34[(value % 34) as usize]);
        value /= 34;
    }
    Some(
        digits
            .iter()
            .rev()
            .take(3)
            .map(|&b| char::from(b))
            .collect(),
    )
}

fn run_macserial(path: &Path, model: &str) -> Result<String, String> {
    run_macserial_with_timeout(path, model, MACSERIAL_TIMEOUT)
}

fn run_macserial_with_timeout(
    path: &Path,
    model: &str,
    timeout: Duration,
) -> Result<String, String> {
    let mut command = Command::new(path);
    command
        .args(["-m", model, "-n", "1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "no stdout pipe".to_string())?;
    // The reader is never joined: a stray grandchild could keep the pipe open
    // long after macserial itself is gone.
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.take(MACSERIAL_OUTPUT_LIMIT).read_to_end(&mut buf);
        let _ = tx.send(buf);
    });

    let status = wait_with_timeout(&mut child, timeout)?;
    if !status.success() {
        return Err(format!("exited with {status}"));
    }
    let output = rx
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| "output not received".to_string())?;
    Ok(String::from_utf8_lossy(&output).into_owned())
}

fn wait_with_timeout(
    child: &mut std::process::Child,
    timeout: Duration,
) -> Result<ExitStatus, String> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("timed out after {}s", timeout.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(err) => return Err(err.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every SMBIOS a Hackintosh build may realistically use.
    const MODELS_UNDER_TEST: &[&str] = &[
        "iMac13,1",
        "iMac13,2",
        "iMac13,3",
        "iMac14,1",
        "iMac14,2",
        "iMac14,3",
        "iMac14,4",
        "iMac15,1",
        "iMac16,1",
        "iMac16,2",
        "iMac17,1",
        "iMac18,1",
        "iMac18,2",
        "iMac18,3",
        "iMac19,1",
        "iMac19,2",
        "iMac20,1",
        "iMac20,2",
        "iMacPro1,1",
        "MacPro6,1",
        "MacPro7,1",
        "MacBookPro11,1",
        "MacBookPro11,2",
        "MacBookPro11,3",
        "MacBookPro11,4",
        "MacBookPro11,5",
        "MacBookPro12,1",
        "MacBookPro13,1",
        "MacBookPro13,2",
        "MacBookPro13,3",
        "MacBookPro14,1",
        "MacBookPro14,2",
        "MacBookPro14,3",
        "MacBookPro15,1",
        "MacBookPro15,2",
        "MacBookPro15,3",
        "MacBookPro15,4",
        "MacBookPro16,1",
        "MacBookPro16,2",
        "MacBookPro16,3",
        "MacBookPro16,4",
        "MacBookAir6,1",
        "MacBookAir6,2",
        "MacBookAir7,1",
        "MacBookAir7,2",
        "MacBookAir8,1",
        "MacBookAir8,2",
        "MacBookAir9,1",
        "Macmini6,1",
        "Macmini6,2",
        "Macmini7,1",
        "Macmini8,1",
        "MacBook8,1",
        "MacBook9,1",
        "MacBook10,1",
    ];

    /// Decode (year, week) of a 12-character serial the way macserial's
    /// `get_serial_info` does.
    fn decode_serial(serial: &str, info: &ModelInfo) -> (u32, u32) {
        let b = serial.as_bytes();
        let year_index = SERIAL_YEAR.iter().position(|&c| c == b[3]).unwrap() as u32;
        let first_year = u32::from(info.years[0]);
        let decade = if first_year >= 2017 && year_index / 2 < 7 {
            2020
        } else {
            2010
        };
        let year = decade + year_index / 2;
        let week = SERIAL_WEEK[1..27]
            .iter()
            .position(|&c| c == b[4])
            .map(|i| i as u32 + 1)
            .unwrap()
            + if year_index % 2 == 1 { 26 } else { 0 };
        (year, week)
    }

    #[test]
    fn checksum_matches_macserial_examples() {
        // Produced by macserial 2.1.8 / quoted in Dortania's guide.
        assert!(mlb_checksum_valid("C02238609J9PHC11M"));
        assert!(mlb_checksum_valid("C02050300J9PHC1UE"));
        assert!(mlb_checksum_valid("C02839303QXH69FJA"));
        assert!(mlb_checksum_valid("CK11902AKBH8X"));
        assert!(!mlb_checksum_valid("C02839303QXH69FJB"));
        assert!(!mlb_checksum_valid("M0000000000000001"));
        assert!(!mlb_checksum_valid("C02839303QXH69FJ"));
        assert!(!mlb_checksum_valid("C0283930OQXH69FJA"));
        assert!(!mlb_checksum_valid(""));
    }

    #[test]
    fn every_hackintosh_model_is_known() {
        for model in MODELS_UNDER_TEST {
            assert!(
                model_info(model).is_some(),
                "{model} missing from the macserial table"
            );
        }
    }

    #[test]
    fn native_identities_are_well_formed() {
        for model in MODELS_UNDER_TEST {
            let info = model_info(model).unwrap();
            for _ in 0..40 {
                let (serial, mlb) = native_serial_and_mlb(model).unwrap();
                assert_eq!(serial.len(), 12, "{model} {serial}");
                assert!(serial_format_valid(&serial), "{model} {serial}");
                assert!(serial.starts_with(info.country), "{model} {serial}");
                assert!(serial.ends_with(info.model_code), "{model} {serial}");
                let (year, week) = decode_serial(&serial, info);
                assert!(
                    info.years.contains(&(year as u16)),
                    "{model} {serial} decodes to {year}"
                );
                assert!((1..=52).contains(&week), "{model} {serial} week {week}");

                assert_eq!(mlb.len(), 17, "{model} {mlb}");
                assert!(mlb_checksum_valid(&mlb), "{model} {mlb}");
                assert!(mlb.starts_with(info.country), "{model} {mlb}");
                assert_eq!(&mlb[11..15], info.board_code, "{model} {mlb}");
                assert!(
                    mlb[3..6].bytes().all(|b| b.is_ascii_digit()),
                    "{model} {mlb}"
                );
            }
        }
    }

    #[test]
    fn legacy_models_get_eleven_and_thirteen_characters() {
        for _ in 0..40 {
            let (serial, mlb) = native_serial_and_mlb("MacPro5,1").unwrap();
            assert_eq!(serial.len(), 11, "{serial}");
            assert!(
                serial.starts_with("CK1") && serial.ends_with("EUH"),
                "{serial}"
            );
            assert!(serial_format_valid(&serial));
            assert_eq!(mlb.len(), 13, "{mlb}");
            assert!(mlb_checksum_valid(&mlb), "{mlb}");
            assert!(mlb[2..5].bytes().all(|b| b.is_ascii_digit()), "{mlb}");
            assert_eq!(&mlb[5..6], "0", "{mlb}");
            assert_eq!(&mlb[9..12], "BH8", "{mlb}");
        }
    }

    #[test]
    fn every_table_model_generates() {
        for info in MODELS {
            for _ in 0..10 {
                let (serial, mlb) =
                    native_serial_and_mlb(info.model).unwrap_or_else(|| panic!("{}", info.model));
                assert!(serial_format_valid(&serial), "{} {serial}", info.model);
                assert!(serial.starts_with(info.country) && serial.ends_with(info.model_code));
                assert!(
                    checksum_ok(&mlb) && (13..=17).contains(&mlb.len()),
                    "{} {mlb}",
                    info.model
                );
                assert!(mlb.contains(info.board_code), "{} {mlb}", info.model);
            }
        }
    }

    #[test]
    fn unknown_model_has_no_native_identity() {
        assert!(native_serial_and_mlb("iMac99,1").is_none());
        assert!(native_serial_and_mlb("").is_none());
    }

    #[test]
    fn model_lookup_ignores_case() {
        assert!(native_serial_and_mlb("imac20,1").is_some());
    }

    #[test]
    fn serial_week_symbols_cover_both_halves() {
        assert_eq!(SERIAL_WEEK[1], b'1');
        assert_eq!(SERIAL_WEEK[26], b'X');
        assert_eq!(SERIAL_WEEK[27], b'1');
        assert_eq!(SERIAL_WEEK[52], b'X');
        assert_eq!(SERIAL_WEEK[53], b'Y');
    }

    #[test]
    fn parses_macserial_output() {
        let out = "C02JF0QEPN5T | C02238609J9PHC11M\n";
        assert_eq!(
            parse_macserial_output(out),
            Some(("C02JF0QEPN5T".to_string(), "C02238609J9PHC11M".to_string()))
        );
        // Legacy pair, CRLF line endings.
        assert_eq!(
            parse_macserial_output("CK120001EUH | CK11902AKBH8X\r\n"),
            Some(("CK120001EUH".to_string(), "CK11902AKBH8X".to_string()))
        );
        // Errors, banners and the -a format are rejected.
        assert_eq!(
            parse_macserial_output(
                "Model id (-1) or name (Foo1,1) is out of valid range [0, 120]!\n"
            ),
            None
        );
        assert_eq!(
            parse_macserial_output("iMac20,1 | C02JF0QEPN5T | C02238609J9PHC11M"),
            None
        );
        assert_eq!(
            parse_macserial_output("C02JF0QEPN5T | C02238609J9PHC11N"),
            None
        );
        assert_eq!(
            parse_macserial_output("C02JF0OEPN5T | C02238609J9PHC11M"),
            None
        );
        assert_eq!(parse_macserial_output(""), None);
        // First valid line wins.
        let noisy =
            "WARN: something\nC02DTFZAPN5T | C02050300J9PHC1UE\nC02JF0QEPN5T | C02238609J9PHC11M\n";
        assert_eq!(
            parse_macserial_output(noisy).map(|p| p.0),
            Some("C02DTFZAPN5T".to_string())
        );
    }

    #[test]
    fn rom_from_mac_accepts_common_notations() {
        assert_eq!(
            rom_from_mac("a4:83:e7:12:34:56").as_deref(),
            Some("A483E7123456")
        );
        assert_eq!(
            rom_from_mac("A4-83-E7-12-34-56").as_deref(),
            Some("A483E7123456")
        );
        assert_eq!(
            rom_from_mac("a483.e712.3456").as_deref(),
            Some("A483E7123456")
        );
        assert_eq!(
            rom_from_mac("A483E7123456").as_deref(),
            Some("A483E7123456")
        );
        assert_eq!(rom_from_mac("00:00:00:00:00:00"), None);
        assert_eq!(rom_from_mac("FF:FF:FF:FF:FF:FF"), None);
        assert_eq!(rom_from_mac("01:00:5E:00:00:01"), None);
        assert_eq!(rom_from_mac("A4:83:E7:12:34"), None);
        assert_eq!(rom_from_mac("G4:83:E7:12:34:56"), None);
    }

    #[test]
    fn random_rom_uses_an_apple_oui() {
        for _ in 0..50 {
            let rom = random_rom();
            assert_eq!(rom.len(), 12);
            let oui = u32::from_str_radix(&rom[..6], 16).unwrap();
            assert!(APPLE_OUIS.contains(&oui), "{rom}");
        }
    }

    #[test]
    fn identity_without_macserial_uses_native_generator() {
        let id = generate_identity("MacPro7,1", None, Some("3C:22:FB:01:02:03")).unwrap();
        assert_eq!(id.model, "MacPro7,1");
        assert!(id.serial.starts_with("F5K") && id.serial.ends_with("P7QM"));
        assert!(mlb_checksum_valid(&id.mlb));
        assert_eq!(id.rom, "3C22FB010203");
        let uuid = uuid::Uuid::parse_str(&id.system_uuid).unwrap();
        assert_eq!(uuid.get_version_num(), 4);
        assert_eq!(id.system_uuid, id.system_uuid.to_uppercase());
        assert_eq!(id.system_uuid.len(), 36);
    }

    #[test]
    fn identity_falls_back_when_macserial_cannot_run() {
        let missing = Path::new("/nonexistent/macserial");
        let id = generate_identity("iMac20,1", Some(missing), None).unwrap();
        assert!(id.serial.ends_with("PN5T"));
        assert_eq!(id.rom.len(), 12);
    }

    #[test]
    fn identity_rejects_bad_models() {
        assert_eq!(
            generate_identity("iMac99,1", None, None).unwrap_err().code,
            "SMBIOS_MODEL_UNKNOWN"
        );
        assert_eq!(
            generate_identity("-a", None, None).unwrap_err().code,
            "SMBIOS_MODEL_INVALID"
        );
        assert_eq!(
            generate_identity("iMac20", None, None).unwrap_err().code,
            "SMBIOS_MODEL_INVALID"
        );
        assert_eq!(
            generate_identity("iMac20,1 ", None, None).unwrap_err().code,
            "SMBIOS_MODEL_INVALID"
        );
    }

    #[cfg(unix)]
    #[test]
    fn identity_uses_macserial_output() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("smbios-gen-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("macserial");
        std::fs::write(&script, "#!/bin/sh\n[ \"$1 $2 $3 $4\" = \"-m iMac20,1 -n 1\" ] || exit 3\necho 'C02JF0QEPN5T | C02238609J9PHC11M'\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let id = generate_identity("iMac20,1", Some(&script), None).unwrap();
        assert_eq!(id.serial, "C02JF0QEPN5T");
        assert_eq!(id.mlb, "C02238609J9PHC11M");

        // Garbage output falls back to the native generator.
        std::fs::write(&script, "#!/bin/sh\necho 'C02JF0QEPN5T | BROKEN'\n").unwrap();
        let id = generate_identity("iMac20,1", Some(&script), None).unwrap();
        assert_ne!(id.mlb, "BROKEN");
        assert!(mlb_checksum_valid(&id.mlb));

        // So does a well-formed pair for another model.
        std::fs::write(
            &script,
            "#!/bin/sh\necho 'F5KCH6ZHP7QM | F5K013200QXK3F7JA'\n",
        )
        .unwrap();
        let id = generate_identity("iMac20,1", Some(&script), None).unwrap();
        assert!(id.serial.ends_with("PN5T"), "{}", id.serial);

        // A hanging tool is killed.
        std::fs::write(&script, "#!/bin/sh\nsleep 30\n").unwrap();
        let started = Instant::now();
        assert!(
            run_macserial_with_timeout(&script, "iMac20,1", Duration::from_millis(300)).is_err()
        );
        assert!(started.elapsed() < Duration::from_secs(10));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
