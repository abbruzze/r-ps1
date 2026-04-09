use std::collections::HashMap;
use crate::core::cdrom::Region;

pub struct Bios {
    pub redump_name: &'static str,
    pub region: Region,
    pub date: &'static str,
}

pub static PS1_BIOS_SET: std::sync::LazyLock<HashMap<&'static str, Bios>> = std::sync::LazyLock::new(|| HashMap::from([
    ("239665b1a3dade1b5a52c06338011044",Bios { redump_name: "ps-10j", region:Region::Japan, date: "1994-09-22"}),
    ("849515939161e62f6b866f6853006780",Bios { redump_name: "ps-11j", region:Region::Japan, date: "1995-01-22"}),
    ("dc2b9bf8da62ec93e868cfd29f0d067d",Bios { redump_name: "ps-20a", region:Region::USA, date: "1995-05-07"}),
    ("54847e693405ffeb0359c6287434cbef",Bios { redump_name: "ps-20e", region:Region::Europe, date: "1995-05-10"}),
    ("da27e8b6dab242d8f91a9b25d80c63b8",Bios { redump_name: "ps-21a", region:Region::USA, date: "1995-07-17"}),
    ("417b34706319da7cf001e76e40136c23",Bios { redump_name: "ps-21e", region:Region::Europe, date: "1995-07-17"}),
    ("cba733ceeff5aef5c32254f1d617fa62",Bios { redump_name: "ps-21j", region:Region::Japan, date: "1995-07-17"}),
    ("924e392ed05558ffdb115408c263dccf",Bios { redump_name: "ps-22a", region:Region::USA, date: "1995-12-04"}),
    ("e2110b8a2b97a8e0b857a45d32f7e187",Bios { redump_name: "ps-22e", region:Region::Europe, date: "1995-12-04"}),
    ("ca5cfc321f916756e3f0effbfaeba13b",Bios { redump_name: "ps-22d", region:Region::Japan, date: "1996-03-06"}),
    ("57a06303dfa9cf9351222dfcbb4a29d9",Bios { redump_name: "ps-22j", region:Region::Japan, date: "1995-12-04"}),
    ("81328b966e6dcf7ea1e32e55e1c104bb",Bios { redump_name: "ps-22j(v)", region:Region::Japan, date: "1995-12-04"}),
    ("490f666e1afb15b7362b406ed1cea246",Bios { redump_name: "ps-30a", region:Region::USA, date: "1996-11-18"}),
    ("32736f17079d0b2b7024407c39bd3050",Bios { redump_name: "ps-30e", region:Region::Europe, date: "1997-01-06"}),
    ("8dd7d5296a650fac7319bce665a6a53c",Bios { redump_name: "ps-30j", region:Region::Japan, date: "1996-09-09"}),
    ("8e4c14f567745eff2f0408c8129f72a6",Bios { redump_name: "ps-40j", region:Region::Japan, date: "1997-08-18"}),
    ("b84be139db3ee6cbd075630aa20a6553",Bios { redump_name: "ps-41a(w)", region:Region::USA, date: "1997-08-18"}),
    ("1e68c231d0896b7eadcad1d7d8e76129",Bios { redump_name: "ps-41a", region:Region::USA, date: "1997-12-16"}),
    ("b9d9a0286c33dc6b7237bb13cd46fdee",Bios { redump_name: "ps-41e", region:Region::Europe, date: "1997-12-16"}),
    ("8abc1b549a4a80954addc48ef02c4521",Bios { redump_name: "psone-43j", region:Region::Japan, date: "2000-03-11"}),
    ("9a09ab7e49b422c007e6d54d7c49b965",Bios { redump_name: "psone-44a", region:Region::USA, date: "2000-03-24"}),
    ("b10f5e0e3d9eb60e5159690680b1e774",Bios { redump_name: "psone-44e", region:Region::Europe, date: "2000-03-24"}),
    ("6e3735ff4c7dc899ee98981385f6f3d0",Bios { redump_name: "psone-45a", region:Region::USA, date: "2000-05-25"}),
    ("de93caec13d1a141a40a79f5c86168d6",Bios { redump_name: "psone-45e", region:Region::Europe, date: "2000-05-25"}),
    ("d8f485717a5237285e4d7c5f881b7f32",Bios { redump_name: "ps2-50j", region:Region::Japan, date: "2000-10-27"}),
]));