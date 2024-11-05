//! Library for playing TFMX music files

#![forbid(unsafe_code)]
#![feature(trait_alias, array_chunks)]
#![warn(
    trivial_casts,
    trivial_numeric_casts,
    clippy::pedantic,
    clippy::nursery,
    missing_docs
)]
// TODO: Fix these lints
#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]
// Not interested in these lints
#![allow(
    clippy::too_many_lines,
    clippy::struct_excessive_bools,
    clippy::redundant_pub_crate,
    clippy::cognitive_complexity
)]

mod header;
mod rendering;
mod song;

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    ops::ControlFlow,
    path::Path,
};

use header::Header;
use rendering::{present_output, try_to_makeblock, AudioCtx};
use song::{Cdb, Hdb, Idb, Mdb, Pdblk};

const TEXT_ROW_LEN: u8 = 40;
const TEXT_ROWS: u8 = 6;
const MAX_SONGS: u8 = 32;
const MAX_CHANNELS: u8 = 8;

#[derive(Debug, Clone)]
struct TfmxCtx {
    danger_freak_hack: bool,
    oops_up_hack: bool,
    single_file: bool,
    ntfhd_offset: u32,
    out_rate: u32,
    editbuf: Box<EditBuf>,
    gemx: bool,
    loops: i32,
    hdb: HdbArr,
    mdb: Mdb,
    cdb: CdbArr,
    pdblk: Pdblk,
    idb: Idb,
    jiffies: i32,
    multimode: bool,
    e_clocks: u32,
}

type CdbArr = [Cdb; 16];
type HdbArr = [Hdb; MAX_CHANNELS as usize];
type EditBuf = [u32; 16384];

impl TfmxCtx {
    fn new(sample_rate: u32) -> Self {
        Self {
            out_rate: sample_rate,
            editbuf: bytemuck::allocation::zeroed_box(),
            danger_freak_hack: false,
            oops_up_hack: false,
            single_file: false,
            ntfhd_offset: 0,
            gemx: false,
            loops: 0,
            hdb: [Hdb::default(); MAX_CHANNELS as usize],
            mdb: Mdb::default(),
            cdb: [Cdb::default(); 16],
            pdblk: Pdblk::default(),
            idb: Idb::default(),
            jiffies: 0,
            multimode: false,
            e_clocks: 14318,
        }
    }

    fn init(&mut self) {
        self.prepare();
        for ch_idx in 0..MAX_CHANNELS as usize {
            self.hdb[ch_idx].cdb_idx = Some(ch_idx);
            self.pdblk.p[ch_idx].num = 0xFF;
            self.pdblk.p[ch_idx].addr = 0;
            song::channel_off(ch_idx & 0xF, &mut self.cdb, &mut self.hdb);
        }
    }
}

fn play_loop(player: &mut TfmxPlayer, mut handler: impl NewDataFn) {
    'do_over: loop {
        let mut audio = AudioCtx::new();
        player.tfmx = player.clean_tfmx.clone();
        player.tfmx.init();
        song::start_song(player.song_idx, 0, &player.header, &mut player.tfmx);
        log::info!("Playing song {}", player.song_idx);
        while try_to_makeblock(
            &player.header,
            &mut audio,
            &mut player.tfmx,
            &player.sample_buf,
            player.ch_on,
        ) != Some(0)
        {
            log::trace!("Making some blocks...");
        }
        while try_to_makeblock(
            &player.header,
            &mut audio,
            &mut player.tfmx,
            &player.sample_buf,
            player.ch_on,
        )
        .is_some()
        {
            match present_output(&mut audio, &mut handler) {
                ControlFlow::Continue(cmd) => {
                    if let Some(cmd) = cmd {
                        match cmd {
                            PlayerCmd::Prev => {
                                player.song_idx = player.song_idx.saturating_sub(1);
                                continue 'do_over;
                            }
                            PlayerCmd::Next => {
                                player.song_idx += 1;
                                continue 'do_over;
                            }
                            PlayerCmd::ToggleBlend => {
                                audio.toggle_blend();
                                let on_off = if audio.is_blend_on() { "on" } else { "off" };
                                log::info!("Stereo blend {on_off}");
                            }
                            PlayerCmd::ToggleCh(ch_idx) => {
                                match player.ch_on.get_mut(ch_idx as usize) {
                                    Some(ch) => {
                                        *ch ^= true;
                                        let on_off = |b| if b { "X" } else { "_" };
                                        log::info!(
                                            "Channel status: {:?}",
                                            player.ch_on.map(on_off)
                                        );
                                    }
                                    None => {
                                        log::warn!("No such channel: {ch_idx}");
                                    }
                                }
                            }
                        }
                    }
                }
                ControlFlow::Break(()) => {
                    log::info!("Stopping playback on request.");
                    break;
                }
            }
        }
        player.song_idx += 1;
    }
}

/// .mdat loading error
#[derive(Debug, thiserror::Error)]
pub enum MdatLoadError {
    /// File doesn't proclaim itself as a TFMX file
    #[error("Not a valid TFMX file (magic mismatch)")]
    MagicMismatch,
    /// I/O error
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),
    /// The edit buffer was too small, not a valid size for TFMX
    #[error("Edit buffer too small: {size}")]
    EditBufferTooSmall {
        /// The size of the edit buffer
        size: usize,
    },
    /// Some kind of preprocessing step went wrong
    #[error("Error occured during preprocessing")]
    PreprocessError,
}

fn load_mdat(mdat_path: &Path, tfmx: &mut TfmxCtx) -> Result<Header, MdatLoadError> {
    let &mut TfmxCtx {
        single_file,
        ntfhd_offset,
        ref mut editbuf,
        ..
    } = tfmx;
    let mut f = File::open(mdat_path)?;
    if single_file {
        f.seek(SeekFrom::Current(i64::from(ntfhd_offset)))?;
    }
    let header = Header::from_reader(&mut f)?;
    let n = f.read(bytemuck::bytes_of_mut(&mut **editbuf))? / size_of::<u32>();
    editbuf[n] = u32::MAX;
    if n < 127 {
        return Err(MdatLoadError::EditBufferTooSmall { size: n });
    }
    for i in 0..128 {
        let z = header.macro_start + i;
        let y = u32::from_be(editbuf[z])
            .checked_sub(0x200)
            .ok_or(MdatLoadError::PreprocessError)?;
        if (y & 3) != 0 || (y >> 2) > n as u32 {
            log::debug!("Counted {i} macros.");
            break;
        }
        editbuf[z] = y >> 2;
    }
    for i in 0..128 {
        let z = header.patt_start + i;
        let y = u32::from_be(editbuf[z])
            .checked_sub(0x200)
            .ok_or(MdatLoadError::PreprocessError)?;
        if (y & 3) != 0 || (y >> 2) > n as u32 {
            log::debug!("Counted {i} patterns.");
            break;
        }
        editbuf[z] = y >> 2;
    }
    let fst_pat = editbuf[header.patt_start];
    let datapoints: &mut [u16; 32768] = bytemuck::cast_mut(&mut **editbuf);
    for datapoint in &mut datapoints[header.track_start * 2..fst_pat as usize * 2] {
        *datapoint = u16::from_be(*datapoint);
    }
    Ok(header)
}

/// Used to build a [`TfmxPlayer`]
pub struct PlayerBuilder {
    mdat_path: String,
    smpl_path: Option<String>,
    song_index: SongIdx,
    sample_rate: u32,
}

/// Error when trying to build a [`TfmxPlayer`]
#[derive(Debug, thiserror::Error)]
pub enum PlayerBuildError {
    /// I/O error
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// Error trying to load the .mdat file
    #[error(".mdat load error: {0}")]
    MDat(#[from] MdatLoadError),
}

impl PlayerBuilder {
    /// Create a new [`PlayerBuilder`] with the specified .mdat path
    pub fn new<S: Into<String>>(mdat_path: S) -> Self {
        Self {
            mdat_path: mdat_path.into(),
            smpl_path: None,
            song_index: 0,
            sample_rate: 44_100,
        }
    }
    /// Specify a file to use as the sample file (Usually .smpl)
    pub fn smpl_file<S: Into<String>>(&mut self, path: S) -> &mut Self {
        self.smpl_path = Some(path.into());
        self
    }
    /// Which subsong to start with
    pub fn starting_subsong(&mut self, idx: u8) -> &mut Self {
        self.song_index = idx;
        self
    }
    /// Specify the sample rate at which we render the song
    pub fn sample_rate(&mut self, rate: u32) -> &mut Self {
        self.sample_rate = rate;
        self
    }
    /// Build the [`TfmxPlayer`].
    ///
    /// # Errors
    ///
    /// Errors on .mdat file loading error
    pub fn build(&mut self) -> Result<TfmxPlayer, PlayerBuildError> {
        let mut tfmx = TfmxCtx::new(self.sample_rate);
        let header = load_mdat(self.mdat_path.as_ref(), &mut tfmx)?;
        let sample_path = self
            .smpl_path
            .take()
            .unwrap_or_else(|| self.mdat_path.replace("mdat.", "smpl."));
        let sample_buf = std::fs::read(sample_path)?;
        Ok(TfmxPlayer {
            clean_tfmx: tfmx.clone(),
            tfmx,
            header,
            sample_buf: bytemuck::cast_vec(sample_buf),
            song_idx: self.song_index,
            ch_on: [true; MAX_CHANNELS as usize],
        })
    }
}

/// TFMX player
pub struct TfmxPlayer {
    clean_tfmx: TfmxCtx,
    tfmx: TfmxCtx,
    header: Header,
    sample_buf: Vec<i8>,
    song_idx: SongIdx,
    ch_on: [bool; MAX_CHANNELS as usize],
}

/// Max value is [`MAX_SONGS`] - 1
type SongIdx = u8;

/// Function for handling new sample data coming from the player
pub trait NewDataFn = FnMut(&[u8]) -> NewDataCtlFlow;
/// Whether to stop playing, or continue playing, with an optional [`PlayerCmd`]
pub type NewDataCtlFlow = ControlFlow<(), Option<PlayerCmd>>;

/// A command telling the player to do something
pub enum PlayerCmd {
    /// Switch to previous subsong
    Prev,
    /// Switch to next subsong
    Next,
    /// Toggle stereo blending
    ToggleBlend,
    /// Mute/unmute audio channel marked by the index
    ToggleCh(u8),
}

impl TfmxPlayer {
    /// Begin playback, using the specified callback to handle sample data produced by the player.
    pub fn play(&mut self, handler: impl NewDataFn) {
        for row in self.header.text_rows() {
            log::info!("{row}");
        }
        play_loop(self, handler);
    }
}
