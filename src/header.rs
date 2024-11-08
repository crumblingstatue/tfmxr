use {
    crate::{MdatLoadError, MAX_CHANNELS, MAX_SONGS, TEXT_ROWS, TEXT_ROW_LEN},
    egui_inspect::derive::Inspect,
};

#[derive(Clone, Copy, Debug, Inspect)]
pub struct Header {
    pub text: [u8; TEXT_ROWS as usize * TEXT_ROW_LEN as usize],
    pub song_starts: [u16; MAX_SONGS as usize],
    pub song_ends: [u16; MAX_SONGS as usize],
    pub song_tempos: [u16; MAX_SONGS as usize],
    pub track_start: usize,
    pub patt_start: usize,
    pub macro_start: usize,
}

impl Header {
    /// # Errors
    ///
    /// WIP
    pub fn from_reader<R: std::io::Read>(reader: &mut R) -> Result<Self, MdatLoadError> {
        let mut img: HeaderImage = bytemuck::zeroed();
        reader.read_exact(bytemuck::bytes_of_mut(&mut img))?;
        if !(&img.magic[0..9] == b"TFMX-SONG"
            || &img.magic[0..9] == b"TFMX_SONG"
            || img.magic.eq_ignore_ascii_case(b"tfmxsong")
            || &img.magic[0..4] == b"TFMX")
        {
            return Err(MdatLoadError::MagicMismatch);
        }
        let track_start = if img.trackstart == 0 {
            0x180
        } else {
            ((u32::from_be(img.trackstart) - 0x200) >> 2) as usize
        };
        let patt_start = if img.pattstart == 0 {
            0x80
        } else {
            ((u32::from_be(img.pattstart) - 0x200) >> 2) as usize
        };
        let macro_start = if img.macrostart == 0 {
            0x100
        } else {
            ((u32::from_be(img.macrostart) - 0x200) >> 2) as usize
        };
        Ok(Self {
            text: img.text,
            song_starts: img.song_starts.map(u16::from_be),
            song_ends: img.song_ends.map(u16::from_be),
            song_tempos: img.song_tempos.map(u16::from_be),
            track_start,
            patt_start,
            macro_start,
        })
    }
    /// Return the rows of text that are valid UTF-8 and aren't empty
    pub fn text_rows(&self) -> impl Iterator<Item = &str> {
        self.text
            .as_ref()
            .array_chunks()
            .filter_map(|chk: &[u8; TEXT_ROW_LEN as usize]| {
                std::str::from_utf8(chk)
                    .ok()
                    .filter(|txt| !txt.trim_matches('\0').trim().is_empty())
            })
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct HeaderImage {
    magic: [u8; 10],
    _pad: [u8; 6],
    text: [u8; TEXT_ROWS as usize * TEXT_ROW_LEN as usize],
    song_starts: [u16; MAX_SONGS as usize],
    song_ends: [u16; MAX_SONGS as usize],
    song_tempos: [u16; MAX_SONGS as usize],
    mute: [i16; MAX_CHANNELS as usize],
    trackstart: u32,
    pattstart: u32,
    macrostart: u32,
    _pad2: [u8; 36],
}
