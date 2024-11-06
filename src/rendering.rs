use {
    crate::{
        header::Header, song::tfmx_irq_in, CdbArr, Hdb, NewDataCtlFlow, TfmxCtx, MAX_CHANNELS,
    },
    std::ops::ControlFlow,
};

const BUFSIZE: usize = 16_384;
const HALFBUFSIZE: usize = BUFSIZE / 2;

pub(crate) struct AudioCtx {
    buf: Box<[i16; BUFSIZE]>,
    bhead: usize,
    btail: usize,
    blocksize: usize,
    multiplier: usize,
    e_rem: usize,
    blend: bool,
    tbuf: Box<TBuf>,
    samples_done: usize,
}

type TBuf = [i32; BUFSIZE];

impl AudioCtx {
    pub(crate) fn new() -> Self {
        let multiplier = 2;
        Self {
            buf: bytemuck::allocation::zeroed_box(),
            bhead: 0,
            btail: 0,
            blocksize: HALFBUFSIZE / multiplier / 2,
            multiplier,
            e_rem: 0,
            blend: true,
            tbuf: bytemuck::allocation::zeroed_box(),
            samples_done: 0,
        }
    }

    pub(crate) fn toggle_blend(&mut self) {
        self.blend ^= true;
    }

    pub(crate) const fn is_blend_on(&self) -> bool {
        self.blend
    }
}

fn mixit(
    iterations: usize,
    tbuf_offset: usize,
    tfmx: &mut TfmxCtx,
    audio: &mut AudioCtx,
    smplbuf: &[i8],
    ch_on: [bool; MAX_CHANNELS as usize],
) {
    if tfmx.multimode {
        if ch_on[4] {
            mix(
                &mut tfmx.hdb[4],
                iterations,
                &mut audio.tbuf[tbuf_offset..],
                smplbuf,
                &mut tfmx.cdb,
            );
        }
        if ch_on[5] {
            mix(
                &mut tfmx.hdb[5],
                iterations,
                &mut audio.tbuf[tbuf_offset..],
                smplbuf,
                &mut tfmx.cdb,
            );
        }
        if ch_on[6] {
            mix(
                &mut tfmx.hdb[6],
                iterations,
                &mut audio.tbuf[tbuf_offset..],
                smplbuf,
                &mut tfmx.cdb,
            );
        }
        if ch_on[7] {
            mix(
                &mut tfmx.hdb[7],
                iterations,
                &mut audio.tbuf[tbuf_offset..],
                smplbuf,
                &mut tfmx.cdb,
            );
        }
    } else if ch_on[3] {
        mix(
            &mut tfmx.hdb[3],
            iterations,
            &mut audio.tbuf[tbuf_offset..],
            smplbuf,
            &mut tfmx.cdb,
        );
    }
    if ch_on[0] {
        mix(
            &mut tfmx.hdb[0],
            iterations,
            &mut audio.tbuf[tbuf_offset..],
            smplbuf,
            &mut tfmx.cdb,
        );
    }
    if ch_on[1] {
        mix(
            &mut tfmx.hdb[1],
            iterations,
            &mut audio.tbuf[HALFBUFSIZE + tbuf_offset..],
            smplbuf,
            &mut tfmx.cdb,
        );
    }
    if ch_on[2] {
        mix(
            &mut tfmx.hdb[2],
            iterations,
            &mut audio.tbuf[HALFBUFSIZE + tbuf_offset..],
            smplbuf,
            &mut tfmx.cdb,
        );
    }
}

/// Perform stereo blending to make headphone listening experience less weird
fn stereo_blend(audio: &mut AudioCtx) {
    for i in 0..audio.samples_done {
        let buf = &mut audio.tbuf[i..];
        let y = ((buf[HALFBUFSIZE] * 11) + ((buf[0]) * 5)) >> 4;
        buf[0] = ((buf[HALFBUFSIZE] * 5) + ((buf[0]) * 11)) >> 4;
        buf[HALFBUFSIZE] = y;
    }
}

fn conv_s16(ctx: &mut AudioCtx) {
    let num = ctx.samples_done;

    // there should always be enough space for conversion since buffer is only
    // filled half so abort in this case. We could wait here instead.
    assert!(available_sound_data(ctx) + (num * ctx.multiplier) < BUFSIZE);

    if ctx.blend {
        stereo_blend(ctx);
    }

    let buf = &mut ctx.buf[ctx.bhead..];
    for i in 0..num {
        buf[i * 2] = ctx.tbuf[i + HALFBUFSIZE] as i16;
        buf[i * 2 + 1] = ctx.tbuf[i] as i16;
        ctx.tbuf[i] = 0;
        ctx.tbuf[i + HALFBUFSIZE] = 0;
    }

    ctx.bhead = (ctx.bhead + (num * ctx.multiplier)) % BUFSIZE;
}

pub(crate) fn try_to_makeblock(
    header: &Header,
    audio: &mut AudioCtx,
    tfmx: &mut TfmxCtx,
    smplbuf: &[i8],
    ch_on: [bool; MAX_CHANNELS as usize],
) -> Option<u32> {
    let mut r = 0;

    while available_sound_data(audio) < BUFSIZE / 2 && tfmx.mdb.player_enable {
        const WHAT: usize = 357_955;

        tfmx_irq_in(header, tfmx);
        let mut nb = (tfmx.e_clocks * (tfmx.out_rate >> 1)) as usize;
        audio.e_rem += nb % WHAT;
        nb /= WHAT;
        if audio.e_rem > WHAT {
            nb += 1;
            audio.e_rem -= WHAT;
        }
        while nb > 0 {
            let mut n = audio.blocksize - audio.samples_done;
            if n > nb {
                n = nb;
            }
            mixit(n, audio.samples_done, tfmx, audio, smplbuf, ch_on);
            audio.samples_done += n;
            nb -= n;

            // convert full blocksize or partial block at end of player
            if audio.samples_done == audio.blocksize || !tfmx.mdb.player_enable {
                conv_s16(audio);
                audio.samples_done = 0;
                r += 1;
            }
        }
    }

    tfmx.mdb.player_enable.then_some(r)
}

const fn available_sound_data(ctx: &AudioCtx) -> usize {
    let l = ctx.bhead.abs_diff(ctx.btail) + BUFSIZE;
    l % BUFSIZE
}

#[must_use]
pub(crate) fn present_output(
    ctx: &mut AudioCtx,
    mut handler: impl crate::NewDataFn,
) -> NewDataCtlFlow {
    let mut total_len = available_sound_data(ctx);
    let mut cmd = None;
    while total_len > 0 {
        let mut len = total_len;
        if ctx.btail + len > BUFSIZE {
            len = BUFSIZE - ctx.btail;
        }
        let end_idx = (ctx.bhead + len).min(ctx.buf.len());
        cmd = handler(&ctx.buf[ctx.bhead..end_idx])?;

        ctx.btail = (ctx.btail + len) % BUFSIZE;

        total_len -= len;
    }
    ControlFlow::Continue(cmd)
}

fn mix(hw: &mut Hdb, iterations: usize, out_buf: &mut [i32], smplbuf: &[i8], cdb_arr: &mut CdbArr) {
    if hw.sample_start >= smplbuf.len() {
        log::error!(
            "mix_add_ov: sample_start out of bounds: {}",
            hw.sample_start
        );
        hw.sample_start = 0;
    }
    let mut end_idx = hw.sbeg + hw.slen as usize;
    if end_idx > smplbuf.len() {
        log::error!(
            "Sample end index out of bounds.\n\
               hw.sbeg: {}\n\
             + hw.slen: {}\n\
             = {} (length of slice is {}).\n\
             Clamping.",
            hw.sbeg,
            hw.slen,
            end_idx,
            smplbuf.len()
        );
        end_idx = smplbuf.len();
    }
    let mut p: &[i8] = &smplbuf[hw.sbeg..end_idx];
    let mut pos: u32 = hw.pos;
    let volume = i32::from(hw.vol.min(0x40));
    let mut delta: u32 = hw.delta;
    let mut len: u32 = u32::from(hw.slen) << FRACTION_BITS;

    /* This used to have (p==&smplbuf).  Broke with GrandMonsterSlam */
    if (((hw.mode) & 1) == 0) || (len < 0x10000) {
        return;
    }
    if (hw.mode & 3) == 1 {
        hw.sbeg = hw.sample_start;
        p = &smplbuf[hw.sample_start..hw.sample_start + hw.sample_len as usize];
        hw.slen = hw.sample_len;
        len = u32::from(hw.sample_len) << FRACTION_BITS;
        pos = 0;
        hw.mode |= 2;
    }

    for sample in out_buf.iter_mut().take(iterations) {
        let pos_real = pos >> FRACTION_BITS;
        let v1 = i32::from(p[pos_real as usize]);
        let v2 = if pos_real + 1 < u32::from(hw.slen) {
            i32::from(p[pos_real as usize + 1])
        } else {
            i32::from(smplbuf[hw.sample_start])
        };
        let base_sample =
            v1 + (((v2 - v1) * (pos & u32::from(FRACTION_MASK)) as i32) >> FRACTION_BITS);
        *sample += volume * base_sample;
        pos += delta;

        if pos < len {
            continue;
        }
        pos -= len;
        p = &smplbuf[hw.sample_start..hw.sample_start + hw.sample_len as usize];
        hw.slen = hw.sample_len;
        len = u32::from(hw.sample_len) << FRACTION_BITS;
        if (len < 0x10000) || ((hw.loop_fn)(hw, cdb_arr) == 0) {
            delta = 0;
            pos = 0;
            hw.slen = 0;
            p = smplbuf;
            break;
        }
    }
    hw.sbeg = usize::abs_diff(p.as_ptr().addr(), smplbuf.as_ptr().addr());
    hw.pos = pos;
    hw.delta = delta;
    if (hw.mode & 4) != 0 {
        (hw.mode = 0);
    }
}

const FRACTION_BITS: u8 = 14;
const INTEGER_MASK: u32 = 0xFFFF_FFFF << FRACTION_BITS;
const FRACTION_MASK: u16 = (!INTEGER_MASK) as u16;
