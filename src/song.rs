use {
    crate::{header::Header, CdbArr, EditBuf, HdbArr, SongIdx, TfmxCtx, MAX_CHANNELS},
    std::cmp::Ordering,
    u32be::U32Be,
};

mod u32be {
    /// 32 bit big-endian integer
    #[derive(Copy, Clone)]
    pub struct U32Be(u32);

    impl U32Be {
        pub const fn from_ne(src: u32) -> Self {
            Self(src)
        }
        pub const fn from_be(src: u32) -> Self {
            Self(u32::from_be(src))
        }
        pub const fn whole(self) -> u32 {
            self.0
        }
        pub const fn hi(self) -> u16 {
            self.0 as u16
        }
        pub const fn hi_signed(self) -> i16 {
            self.0 as i16
        }
        pub fn byte<const IDX: u8>(self) -> u8 {
            let arr: &[u8; 4] = { bytemuck::cast_ref(&self.0) };
            arr[const { 3 - IDX as usize }]
        }
        pub fn byte_mut<const IDX: u8>(&mut self) -> &mut u8 {
            let arr: &mut [u8; 4] = { bytemuck::cast_mut(&mut self.0) };
            &mut arr[const { 3 - IDX as usize }]
        }
    }
}

impl TfmxCtx {
    pub(crate) fn prepare(&mut self) {
        self.mdb.player_enable = false;
        for (i, (hw, c)) in self.hdb.iter_mut().zip(self.cdb.iter_mut()).enumerate() {
            hw.mode = 0;
            hw.vol = 0;
            hw.cdb_idx = Some(i); /* wait on dma */
            hw.sbeg = 0;
            hw.sample_start = 0;
            hw.sample_len = 2;
            hw.slen = 2;
            hw.loop_fn = Some(loop_off);
            c.hw_idx = i;
            c.macro_wait = 0;
            c.macro_run = 0;
            c.sfx_flag = 0;
            c.cur_vol = 0;
            c.sfx_flag = 0;
            c.sfx_code = 0;
            c.save_addr = 0;
            c.loop_ = -1;
            c.new_style_macro = u8::MAX;
            c.sfx_lock_time = -1;
            c.save_len = 2;
        }
    }
}

#[expect(clippy::too_many_arguments)]
fn run_macro(
    c_idx: usize,
    editbuf: &EditBuf,
    gemx: bool,
    danger_freak_hack: bool,
    cdb_arr: &mut CdbArr,
    multimode: bool,
    macros: &[u32],
    idb: &mut Idb,
    hdb_arr: &mut HdbArr,
) {
    #[derive(Debug)]
    enum Action {
        HwMod,
        CLoop1,
        CPeriod(u8),
        CLoop2,
    }

    cdb_arr[c_idx].macro_wait = 0;
    loop {
        let c = &mut cdb_arr[c_idx];
        let macro_step = c.macro_step;
        c.macro_step = c.macro_step.wrapping_add(1);
        let mut word =
            U32Be::from_be(editbuf[(c.macro_ptr).wrapping_add(u32::from(macro_step)) as usize]);
        let byte_0 = word.byte::<0>();
        *word.byte_mut::<0>() = 0;
        let action = match byte_0 {
            0 => {
                c.add_begin_time = 0;
                c.porta_rate = 0;
                c.vib_reset = 0;
                c.env_reset = 0;
                if gemx {
                    if word.byte::<2>() != 0 {
                        c.cur_vol = word.byte::<3>() as i8;
                    } else {
                        c.cur_vol = (i32::from(word.byte::<3>()) + i32::from(c.velocity) * 3) as i8;
                    }
                }
                Action::HwMod
            }
            19 => Action::HwMod,
            1 => {
                c.efx_run = word.byte::<1>() as i8;
                let hw = &mut hdb_arr[c.hw_idx];
                hw.mode = 1;
                if c.new_style_macro == 0 || danger_freak_hack {
                    hw.sample_start = c.save_addr as usize;
                    hw.sample_len = if c.save_len != 0 { c.save_len << 1 } else { 0 };
                    hw.sbeg = hw.sample_start;
                    hw.slen = hw.sample_len;
                    hw.pos = 0;
                    hw.mode |= 2;
                    continue;
                }
                continue;
            }
            2 => {
                c.add_begin_time = 0;
                c.curr_addr = word.whole();
                c.save_addr = c.curr_addr;
                continue;
            }
            17 => {
                c.add_begin_reset = word.byte::<1>();
                c.add_begin_time = c.add_begin_reset;
                c.add_begin = i32::from(word.hi_signed());
                c.curr_addr = c.curr_addr.wrapping_add(c.add_begin as u32);
                c.save_addr = c.curr_addr;
                continue;
            }
            3 => {
                c.curr_len = word.hi();
                c.save_len = c.curr_len;
                continue;
            }
            18 => {
                c.curr_len = (i32::from(c.curr_len) + i32::from(word.hi_signed())) as u16;
                c.save_len = c.curr_len;
                continue;
            }
            4 => {
                if i32::from(word.byte::<1>()) & 0x1 != 0 {
                    let really_wait = c.really_wait;
                    c.really_wait = c.really_wait.wrapping_add(1);
                    if really_wait != 0 {
                        return;
                    }
                }
                c.macro_wait = word.hi();
                if c.new_style_macro == 0 {
                    c.new_style_macro = 0xff;
                    continue;
                }
                return;
            }
            26 => {
                let hw = &mut hdb_arr[c.hw_idx];
                hw.loop_fn = Some(loop_on);
                hw.cdb_idx = Some(c_idx);
                c.wait_dma_count = word.hi();
                c.macro_run = 0;
                if c.new_style_macro == 0 {
                    c.new_style_macro = 0xff;
                    continue;
                }
                return;
            }
            28 => {
                if i32::from(c.curr_note) > i32::from(word.byte::<1>()) {
                    c.macro_step = word.hi();
                }
                continue;
            }
            29 => {
                if i32::from(c.cur_vol) > i32::from(word.byte::<1>()) {
                    c.macro_step = word.hi();
                }
                continue;
            }
            16 => {
                if c.key_up == 0 {
                    continue;
                }
                Action::CLoop1
            }
            5 => Action::CLoop1,
            7 => {
                c.macro_run = 0;
                return;
            }
            13 => {
                if i32::from(word.byte::<2>()) != 0xfe {
                    let vol = (i32::from(c.velocity) * 3 + i32::from(word.byte::<3>())) as i8;
                    if vol > 0x40 {
                        c.cur_vol = 0x40;
                    } else {
                        c.cur_vol = vol;
                    }
                    continue;
                }
                continue;
            }
            14 => {
                if i32::from(word.byte::<2>()) != 0xfe {
                    c.cur_vol = word.byte::<3>() as i8;
                    continue;
                }
                continue;
            }
            33 => {
                *word.byte_mut::<0>() = c.curr_note;
                *word.byte_mut::<2>() =
                    (i32::from(word.byte::<2>()) | i32::from(c.velocity) << 4) as u8;
                note_port(word.whole(), cdb_arr, multimode, danger_freak_hack, macros);
                continue;
            }
            31 => Action::CPeriod(c.prev_note),
            8 => Action::CPeriod(c.curr_note),
            9 => Action::CPeriod(0),
            23 => {
                c.dest_period = word.hi();
                if c.porta_rate == 0 {
                    c.cur_period = word.hi();
                }
                continue;
            }
            11 => {
                c.porta_reset = word.byte::<1>();
                c.porta_time = 1;
                if c.porta_rate == 0 {
                    c.porta_per = c.dest_period;
                }
                c.porta_rate = word.hi_signed();
                continue;
            }
            12 => {
                c.vib_reset = word.byte::<1>();
                c.vib_time = (i32::from(c.vib_reset) >> 1) as u8;
                c.vib_width = word.byte::<3>() as i8;
                c.vib_flag = 1;
                if c.porta_rate == 0 {
                    c.cur_period = c.dest_period;
                    c.vib_offset = 0;
                }
                continue;
            }
            15 => {
                c.env_time = word.byte::<2>();
                c.env_reset = c.env_time;
                c.env_end_vol = word.byte::<3>() as i8;
                c.env_rate = word.byte::<1>();
                continue;
            }
            10 => {
                c.add_begin_time = 0;
                c.porta_rate = i16::from(c.add_begin_time);
                c.vib_reset = c.porta_rate as u8;
                c.env_reset = c.vib_reset;
                continue;
            }
            20 => {
                if c.key_up == 0 {
                    c.loop_ = 0;
                }
                if c.loop_ == 0 {
                    c.loop_ = -1;
                    continue;
                }
                if c.loop_ == -1 {
                    c.loop_ = i16::from(word.byte::<3>()) - 1;
                } else {
                    c.loop_ -= 1;
                }
                c.macro_step = c.macro_step.wrapping_sub(1);
                return;
            }
            21 => {
                c.return_ptr = c.macro_ptr as u16;
                c.return_step = c.macro_step;
                Action::CLoop2
            }
            6 => Action::CLoop2,
            22 => {
                c.macro_ptr = u32::from(c.return_ptr);
                c.macro_step = c.return_step;
                continue;
            }
            24 => {
                c.save_addr = c.save_addr.wrapping_add(u32::from(word.hi()) & 0xfffe);
                c.save_len -= word.hi() >> 1;
                c.curr_len = c.save_len;
                c.curr_addr = c.save_addr;
                continue;
            }
            25 => {
                c.add_begin_time = 0;
                c.curr_addr = 0;
                c.save_addr = c.curr_addr;
                c.curr_len = 1;
                c.save_len = c.curr_len;
                continue;
            }
            32 => {
                idb.cue[(i32::from(word.byte::<1>()) & 0x3) as usize] = word.hi();
                continue;
            }
            34 => {
                c.add_begin_time = 0;
                c.curr_addr = word.whole();
                continue;
            }
            _ => {
                log::warn!("Unknown 'byte_0': {byte_0}");
                continue;
            }
        };
        match action {
            Action::HwMod => {
                let hw = &mut hdb_arr[c.hw_idx];
                hw.loop_fn = Some(loop_off);
                if word.byte::<1>() == 0 {
                    hw.mode = 0;
                    if c.new_style_macro != 0 {
                        hw.slen = 0;
                    }
                } else {
                    hw.mode = (i32::from(hw.mode) | 4) as u8;
                    c.new_style_macro = 0;
                    return;
                }
            }
            Action::CPeriod(a) => {
                let note_idx = ((a.wrapping_add(word.byte::<1>())) & 0x3f) as usize;
                let note = u32::from(NOTEVALS[note_idx]);
                let period =
                    (note * (0x100 + u32::from(c.fine_tune) + u32::from(word.byte::<3>()))) >> 8;
                c.dest_period = period as u16;
                if c.porta_rate == 0 {
                    c.cur_period = period as u16;
                }
                if c.new_style_macro == 0 {
                    c.new_style_macro = 0xff;
                } else {
                    return;
                }
            }
            Action::CLoop1 => {
                let loop_ = c.loop_;
                c.loop_ -= 1;
                if loop_ == 0 {
                    continue;
                }
                if i32::from(c.loop_) < 0 {
                    c.loop_ = i16::from(word.byte::<1>()) - 1;
                }
                c.macro_step = word.hi();
            }
            Action::CLoop2 => {
                c.macro_num = macros[usize::from(word.byte::<1>())] as u16;
                c.macro_ptr = u32::from(c.macro_num);
                c.macro_step = word.hi();
                c.loop_ = -1;
            }
        }
    }
}

#[expect(clippy::too_many_arguments)]
fn get_track_step(
    track_start: usize,
    pdb: &mut Pdblk,
    loops: &mut i32,
    jiffies: &mut i32,
    mdb: &mut Mdb,
    e_clocks: &mut u32,
    editbuf: &EditBuf,
    multimode: &mut bool,
    patterns_idx: usize,
) {
    loop {
        if pdb.curr_pos == pdb.first_pos && *loops <= 0 {
            if *loops < 0 {
                mdb.player_enable = false;
                return;
            }
            *loops -= 1;
        }
        let l: &[u16] = bytemuck::cast_slice(
            &editbuf[track_start.wrapping_add(usize::from(pdb.curr_pos) * 4)..],
        );
        *jiffies = 0;
        if l[0] == 0xeffe {
            match l[1] {
                0 => {
                    mdb.player_enable = false;
                    return;
                }
                1 => {
                    if *loops != 0 {
                        *loops -= 1;
                        if *loops == 0 {
                            mdb.player_enable = false;
                            return;
                        }
                    }
                    let track_loop = mdb.track_loop;
                    mdb.track_loop -= 1;
                    if track_loop == 0 {
                        mdb.track_loop = -1;
                        pdb.curr_pos = pdb.curr_pos.wrapping_add(1);
                    } else {
                        if mdb.track_loop < 0 {
                            mdb.track_loop = l[3] as i16;
                        }
                        pdb.curr_pos = l[2];
                    }
                }
                2 => {
                    pdb.prescale = l[2];
                    mdb.speed_cnt = pdb.prescale;
                    let x;
                    if l[3] & 0xf200 == 0 && {
                        x = i32::from(i32::from(l[3]) & 0x1ff > 0xf);
                        x != 0
                    } {
                        *e_clocks = (0x001b_51f8 / x) as u32;
                        mdb.cia_save = *e_clocks as u16;
                    }
                    pdb.curr_pos = pdb.curr_pos.wrapping_add(1);
                }
                3 => {
                    let mut x = i32::from(l[3]);
                    if x & 0x8000 == 0 {
                        x = if i32::from(x as i8) < -0x20 {
                            -0x20
                        } else {
                            i32::from(x as i8)
                        };
                        *e_clocks = (14318 * (x + 100) / 100) as u32;
                        mdb.cia_save = *e_clocks as u16;
                        *multimode = true;
                    }
                    pdb.curr_pos = pdb.curr_pos.wrapping_add(1);
                }
                4 => {
                    do_fade(i32::from(l[2]) & 0xff, i32::from(l[3]) & 0xff, mdb);
                    pdb.curr_pos = pdb.curr_pos.wrapping_add(1);
                }
                _ => {
                    pdb.curr_pos = pdb.curr_pos.wrapping_add(1);
                }
            }
        } else {
            for (pdb, l) in pdb.p.iter_mut().zip(l) {
                pdb.xpose = (l & 0xff) as i8;
                pdb.num = (l >> 8) as u8;
                let y = pdb.num;
                if y < 0x80 {
                    pdb.step = 0;
                    pdb.wait = 0;
                    pdb.loop_ = 0xffff;
                    let patterns = &editbuf[patterns_idx..];
                    pdb.addr = patterns[usize::from(y)];
                }
            }
            return;
        }
    }
}

#[expect(clippy::too_many_arguments)]
fn do_track(
    p_idx: usize,
    track_start: usize,
    cdb: &mut CdbArr,
    editbuf: &EditBuf,
    multimode: &mut bool,
    danger_freak_hack: bool,
    macros: &[u32],
    pdb: &mut Pdblk,
    loops: &mut i32,
    jiffies: &mut i32,
    mdb: &mut Mdb,
    e_clocks: &mut u32,
    patterns_idx: usize,
    idb: &mut Idb,
    hdb_arr: &mut HdbArr,
) -> bool {
    let patterns = &editbuf[patterns_idx..];
    let p: &mut Pdb = &mut pdb.p[p_idx];
    if p.num == 0xFE {
        p.num += 1;
        channel_off((p.xpose & 0xF) as usize, cdb, hdb_arr);
        return false;
    }
    if p.addr == 0 {
        return false;
    }
    if p.num >= 0x90 {
        return false;
    }
    let p_wait = p.wait;
    p.wait = p.wait.wrapping_sub(1);
    if p_wait != 0 {
        return false;
    }
    loop {
        let p: &mut Pdb = &mut pdb.p[p_idx];
        let p_step = p.step;
        p.step += 1;
        let mut word = U32Be::from_be(editbuf[p.addr as usize + p_step as usize]);
        let mut t = word.byte::<0>();
        if t < 0xF0 {
            if (t & 0xC0) == 0x80 {
                p.wait = word.byte::<3>();
                *word.byte_mut::<3>() = 0;
            }
            *word.byte_mut::<0>() = t.wrapping_add_signed(p.xpose) & 0x3F;
            if (t & 0xC0) == 0xC0 {
                {
                    *word.byte_mut::<0>() |= 0xC0;
                };
            }
            {
                note_port(word.whole(), cdb, *multimode, danger_freak_hack, macros);
            }
            if (t & 0xC0) == 0x80 {
                return false;
            }
            continue;
        }
        match t & 0xF {
            15 => {
                // NOP
            }

            0 => {
                // End
                p.num = 0xFF;
                pdb.curr_pos = if pdb.curr_pos == pdb.last_pos {
                    pdb.first_pos
                } else {
                    pdb.curr_pos + 1
                };
                get_track_step(
                    track_start,
                    pdb,
                    loops,
                    jiffies,
                    mdb,
                    e_clocks,
                    editbuf,
                    multimode,
                    patterns_idx,
                );
                return true;
            }
            1 => 'blk: {
                if p.loop_ == 0 {
                    p.loop_ = 0xFFFF;
                    break 'blk;
                } else if p.loop_ == 0xFFFF {
                    p.loop_ = u16::from(word.byte::<1>());
                }
                p.loop_ = p.loop_.wrapping_sub(1);
                p.step = word.hi();
            }

            8 => {
                // GsPt
                p.ro_addr = p.addr as u16;
                p.ro_step = p.step;
                // repeated fallthrough code
                p.addr = patterns[usize::from(word.byte::<1>())];
                p.step = word.hi();
            }

            2 => {
                // Cont
                p.addr = patterns[usize::from(word.byte::<1>())];
                p.step = word.hi();
            }

            3 => {
                // Wait
                p.wait = word.byte::<1>();
                return false;
            }

            14 => {
                // StCu
                mdb.play_patt_flag = 0;
                // repeated fallthrough code
                p.num = 0xFF;
                return false;
            }

            4 => {
                // Stop
                p.num = 0xFF;
                return false;
            }

            5 | 6 | 7 | 12 => {
                // Kup^ | Vibr | Enve | Lock
                note_port(word.whole(), cdb, *multimode, danger_freak_hack, macros);
            }

            9 => {
                // RoPt
                p.addr = u32::from(p.ro_addr);
                p.step = p.ro_step;
            }

            10 => {
                // Fade
                do_fade(
                    i32::from(word.byte::<1>()),
                    i32::from(word.byte::<3>()),
                    mdb,
                );
            }

            13 => {
                // Cue
                idb.cue[(word.byte::<1>() & 0x03) as usize] = word.hi();
            }

            11 => {
                // PPat
                t = word.byte::<2>() & 0x07;
                pdb.p[t as usize].num = word.byte::<1>();
                pdb.p[t as usize].addr = patterns[usize::from(word.byte::<1>())];
                pdb.p[t as usize].xpose = word.byte::<3>() as i8;
                pdb.p[t as usize].step = 0;
                pdb.p[t as usize].wait = 0;
                pdb.p[t as usize].loop_ = 0xFFFF;
            }
            // We covered all possible values for the bitmask
            _ => unreachable!(),
        }
    }
}

static NOTEVALS: [u16; 64] = [
    0x6AE, 0x64E, 0x5F4, 0x59E, 0x54D, 0x501, 0x4B9, 0x475, 0x435, 0x3F9, 0x3C0, 0x38C, 0x358,
    0x32A, 0x2FC, 0x2D0, 0x2A8, 0x282, 0x25E, 0x23B, 0x21B, 0x1FD, 0x1E0, 0x1C6, 0x1AC, 0x194,
    0x17D, 0x168, 0x154, 0x140, 0x12F, 0x11E, 0x10E, 0x0FE, 0x0F0, 0x0E3, 0x0D6, 0x0CA, 0x0BF,
    0x0B4, 0x0AA, 0x0A0, 0x097, 0x08F, 0x087, 0x07F, 0x078, 0x071, 0x0D6, 0x0CA, 0x0BF, 0x0B4,
    0x0AA, 0x0A0, 0x097, 0x08F, 0x087, 0x07F, 0x078, 0x071, 0x0D6, 0x0CA, 0x0BF, 0x0B4,
];

fn note_port(
    i: u32,
    cdb_arr: &mut CdbArr,
    multimode: bool,
    danger_freak_hack: bool,
    macros: &[u32],
) {
    let word = U32Be::from_ne(i);
    let c = &mut cdb_arr[(word.byte::<2>() & (if multimode { 7 } else { 3 })) as usize];
    if word.byte::<0>() == 0xFC {
        /* lock */
        c.sfx_flag = word.byte::<1>();
        c.sfx_lock_time = i16::from(word.byte::<3>());
        return;
    }
    if c.sfx_flag != 0 {
        return;
    }
    if word.byte::<0>() < 0xC0 {
        if danger_freak_hack {
            c.fine_tune = 0;
        } else {
            c.fine_tune = word.byte::<3>();
        }

        c.velocity = (word.byte::<2>() >> 4) & 0xF;
        c.prev_note = c.curr_note;
        c.curr_note = word.byte::<0>();
        c.really_wait = 1;
        c.new_style_macro = 0xFF;
        c.macro_num = u16::from(word.byte::<1>());
        c.macro_ptr = macros[c.macro_num as usize];

        c.macro_step = 0;
        c.efx_run = 0;
        c.macro_wait = 0;

        c.key_up = 1;
        c.loop_ = -1;
        c.macro_run = -1;
    } else if word.byte::<0>() < 0xF0 {
        c.porta_reset = word.byte::<1>();
        c.porta_time = 1;
        if c.porta_rate == 0 {
            c.porta_per = c.dest_period;
        }
        c.porta_rate = i16::from(word.byte::<3>());
        c.curr_note = word.byte::<0>() & 0x3F;
        c.dest_period = NOTEVALS[c.curr_note as usize];
    } else {
        match word.byte::<0>() {
            0xF7 =>
            /* enve */
            {
                c.env_rate = word.byte::<1>();
                c.env_reset = (word.byte::<2>() >> 4) + 1;
                c.env_time = (word.byte::<2>() >> 4) + 1;
                c.env_end_vol = word.byte::<3>() as i8;
            }

            0xF6 =>
            /* vibr */
            {
                c.vib_reset = (word.byte::<1>() & 0xFE) >> 1;
                c.vib_time = c.vib_reset;
                c.vib_width = word.byte::<3>() as i8;
                c.vib_flag = 1; /* ?! */
                c.vib_offset = 0;
            }

            0xF5 =>
            /* kup^ */
            {
                c.key_up = 0;
            }
            _ => todo!(),
        }
    }
}

fn do_effects(c: &mut Cdb, mdb: &mut Mdb) {
    let mut a: i32 = 0;
    if c.efx_run < 0 {
        return;
    }
    if c.efx_run == 0 {
        c.efx_run = 1;
        return;
    }
    if c.add_begin_time != 0 {
        c.curr_addr = c.curr_addr.wrapping_add(c.add_begin as u32);
        c.save_addr = c.curr_addr;
        c.add_begin_time -= 1;
        if c.add_begin_time == 0 {
            c.add_begin = -c.add_begin;
            c.add_begin_time = c.add_begin_reset;
        }
    }
    if c.vib_reset != 0 {
        c.vib_offset += i16::from(c.vib_width);
        a = i32::from(c.vib_offset);
        a = (i32::from(c.dest_period) * (0x800 + a)) >> 11;
        if c.porta_rate == 0 {
            c.cur_period = a as u16;
        }
        c.vib_time -= 1;
        if c.vib_time == 0 {
            c.vib_time = c.vib_reset;
            c.vib_width = -c.vib_width;
        }
    }
    c.porta_time = c.porta_time.wrapping_sub(1);
    if (c.porta_rate != 0) && ((c.porta_time) == 0) {
        c.porta_time = c.porta_reset;
        match c.porta_per.cmp(&c.dest_period) {
            Ordering::Less => {
                a = (i32::from(c.porta_per) * (256 + i32::from(c.porta_rate))) >> 8;
                if a >= i32::from(c.dest_period) {
                    c.porta_rate = 0;
                }
            }
            Ordering::Equal => {
                c.porta_rate = 0;
            }
            Ordering::Greater => {
                a = (i32::from(c.porta_per) * (256 - i32::from(c.porta_rate)) - 128) >> 8;
                if a <= i32::from(c.dest_period) {
                    c.porta_rate = 0;
                }
            }
        }
        if c.porta_rate == 0 {
            a = i32::from(c.dest_period);
        }
        c.cur_period = a as u16;
        c.porta_per = a as u16;
    }
    let env_time: u8 = c.env_time;
    c.env_time = c.env_time.wrapping_sub(1);
    if (c.env_reset != 0) && env_time == 0 {
        c.env_time = c.env_reset;
        match c.cur_vol.cmp(&c.env_end_vol) {
            Ordering::Less => {
                c.cur_vol += c.env_rate as i8;
                if c.env_end_vol < c.cur_vol {
                    c.env_reset = 0;
                }
            }
            Ordering::Equal => { /* do nothing */ }
            Ordering::Greater => {
                if c.cur_vol < c.env_rate as i8 {
                    c.env_reset = 0;
                } else {
                    c.cur_vol -= c.env_rate as i8;
                }
                if c.env_end_vol > c.cur_vol {
                    c.env_reset = 0;
                }
            }
        }
        if c.env_reset == 0 {
            c.env_reset = 0;
            c.env_time = 0;
            c.cur_vol = c.env_end_vol;
        }
    }
    mdb.fade_time = mdb.fade_time.wrapping_sub(1);
    if (mdb.fade_slope != 0) && (mdb.fade_time == 0) {
        mdb.fade_time = mdb.fade_reset;
        mdb.master_vol += mdb.fade_slope;
        if mdb.fade_dest == mdb.master_vol {
            mdb.fade_slope = 0;
        }
    }
}

fn do_fade(sp: i32, dv: i32, mdb: &mut Mdb) {
    mdb.fade_dest = dv as i8;
    mdb.fade_reset = sp as i8;
    mdb.fade_time = sp as i8;
    if sp == 0 || (mdb.master_vol == sp as i8) {
        mdb.master_vol = dv as i8;
        mdb.fade_slope = 0;
        return;
    }
    mdb.fade_slope = if mdb.master_vol > mdb.fade_dest {
        -1
    } else {
        1
    };
}

fn do_macro(cdb_idx: usize, macros_start: usize, tfmx: &mut TfmxCtx) {
    let &mut TfmxCtx {
        danger_freak_hack,
        out_rate,
        ref editbuf,
        gemx,
        ref mut mdb,
        ref mut cdb,
        ref mut idb,
        multimode,
        ref mut hdb,
        ..
    } = tfmx;
    let c = &mut cdb[cdb_idx];
    /* locking */
    if c.sfx_lock_time >= 0 {
        c.sfx_lock_time -= 1;
    } else {
        c.sfx_flag = 0;
        c.sfx_priority = 0;
    }

    let sfx_code = c.sfx_code;
    if sfx_code != 0 {
        c.sfx_flag = 0;
        c.sfx_code = 0;
        note_port(
            sfx_code,
            cdb,
            multimode,
            danger_freak_hack,
            &editbuf[macros_start..],
        );
        let c = &mut cdb[cdb_idx];
        c.sfx_flag = c.sfx_priority;
    }
    let c = &mut cdb[cdb_idx];
    let n_run: i32 = i32::from(c.macro_run);
    let n_wait: i32 = i32::from(c.macro_wait);
    c.macro_wait = c.macro_wait.wrapping_sub(1);

    if (n_run != 0) && n_wait == 0 {
        run_macro(
            cdb_idx,
            editbuf,
            gemx,
            danger_freak_hack,
            cdb,
            multimode,
            &editbuf[macros_start..],
            idb,
            hdb,
        );
    }
    let c = &mut cdb[cdb_idx];
    do_effects(c, mdb);
    let hw = &mut hdb[c.hw_idx];
    if c.cur_period != 0 {
        hw.delta = (3_579_545 << 9) / ((u32::from(c.cur_period) * out_rate) >> 5);
    } else {
        hw.delta = 0;
    }
    hw.sample_start = c.save_addr as usize;
    if c.save_len != 0 {
        hw.sample_len = c.save_len << 1;
    } else {
        hw.sample_len = 0;
    }
    if (hw.mode & 3) == 1 {
        hw.sbeg = hw.sample_start;
        hw.slen = hw.sample_len;
    }
    hw.vol = ((i32::from(c.cur_vol) * i32::from(mdb.master_vol)) >> 6) as u8;
}

fn do_tracks(tfmx: &mut TfmxCtx, track_start: usize, macros_start: usize, patterns_start: usize) {
    let &mut TfmxCtx {
        danger_freak_hack,
        oops_up_hack,
        ref editbuf,
        ref mut loops,
        ref mut mdb,
        ref mut cdb,
        pdblk: ref mut pdb,
        ref mut idb,
        ref mut jiffies,
        ref mut multimode,
        ref mut e_clocks,
        ref mut hdb,
        ..
    } = tfmx;
    *jiffies += 1;
    let ready = mdb.speed_cnt == 0;
    mdb.speed_cnt = mdb.speed_cnt.wrapping_sub(1);
    if ready {
        mdb.speed_cnt = pdb.prescale;
        /* sortof fix Oops Up tempo */
        if oops_up_hack {
            mdb.speed_cnt = 5;
        }

        let mut x = 0;
        while x < usize::from(MAX_CHANNELS) {
            if do_track(
                x,
                track_start,
                cdb,
                editbuf,
                multimode,
                danger_freak_hack,
                &editbuf[macros_start..],
                pdb,
                loops,
                jiffies,
                mdb,
                e_clocks,
                patterns_start,
                idb,
                hdb,
            ) {
                x = 0;
                continue;
            }
            x += 1;
        }
    }
}

fn do_all_macros(tfmx: &mut TfmxCtx, macros_start: usize) {
    do_macro(0, macros_start, tfmx);
    do_macro(1, macros_start, tfmx);
    do_macro(2, macros_start, tfmx);
    if tfmx.multimode {
        do_macro(4, macros_start, tfmx);
        do_macro(5, macros_start, tfmx);
        do_macro(6, macros_start, tfmx);
        do_macro(7, macros_start, tfmx);
    } /* else -- DoMacro(3) should always run so fade speed is right */
    do_macro(3, macros_start, tfmx);
}

pub(crate) fn channel_off(cdb_idx: usize, cdb_arr: &mut CdbArr, hdb_arr: &mut HdbArr) {
    let c = &mut cdb_arr[cdb_idx];
    if c.sfx_flag == 0 {
        c.add_begin_time = 0;
        c.add_begin_reset = 0;
        c.macro_run = 0;
        c.new_style_macro = 0xFF;
        c.save_addr = 0;
        c.cur_vol = 0;
        c.save_len = 1;
        c.curr_len = 1;
        let hw = &mut hdb_arr[c.hw_idx];
        hw.mode = 0;
        hw.vol = 0;
        hw.loop_fn = Some(loop_off);
        hw.cdb_idx = Some(cdb_idx);
    }
}

pub(crate) fn loop_off(_hdb: &mut Hdb, _cdb_arr: &mut CdbArr) -> i32 {
    1
}

fn loop_on(hdb: &mut Hdb, cdb_arr: &mut CdbArr) -> i32 {
    let Some(cdb_idx) = hdb.cdb_idx else {
        return 1;
    };
    let c = &mut cdb_arr[cdb_idx];
    let wait_dma = c.wait_dma_count;
    c.wait_dma_count = c.wait_dma_count.wrapping_sub(1);
    if wait_dma != 0 {
        return 1;
    }
    hdb.loop_fn = Some(loop_off);
    c.macro_run = -1;
    1
}

pub(crate) fn tfmx_irq_in(header: &Header, tfmx: &mut TfmxCtx) {
    if !tfmx.mdb.player_enable {
        return;
    }
    do_all_macros(tfmx, header.macro_start);
    if tfmx.mdb.curr_song >= 0 {
        do_tracks(
            tfmx,
            header.track_start,
            header.macro_start,
            header.patt_start,
        );
    }
}

pub(crate) fn start_song(song: SongIdx, mode: i32, header: &Header, tfmx: &mut TfmxCtx) {
    let &mut TfmxCtx {
        ref editbuf,
        ref mut loops,
        ref mut mdb,
        pdblk: ref mut pdb,
        ref mut jiffies,
        ref mut multimode,
        ref mut e_clocks,
        ..
    } = tfmx;
    mdb.player_enable = false; /* sort of locking mechanism */
    mdb.master_vol = 0x40;
    mdb.fade_slope = 0;
    mdb.track_loop = -1;
    mdb.play_patt_flag = 0;
    *e_clocks = 14318; /* assume 125bpm, NTSC timing */
    mdb.cia_save = 14318;
    if mode != 2 {
        pdb.first_pos = header.song_starts[song as usize];
        pdb.curr_pos = header.song_starts[song as usize];
        pdb.last_pos = header.song_ends[song as usize];
        let tempo = header.song_tempos[song as usize];
        if tempo >= 0x10 {
            *e_clocks = 0x001B_51F8 / u32::from(tempo);
            mdb.cia_save = *e_clocks as u16;
            pdb.prescale = 0;
        } else {
            pdb.prescale = tempo;
        }
    }
    for pdb in &mut pdb.p {
        pdb.addr = 0;
        pdb.num = 0xFF;
        pdb.xpose = 0;
        pdb.step = 0;
    }
    if mode != 2 {
        get_track_step(
            header.track_start,
            pdb,
            loops,
            jiffies,
            mdb,
            e_clocks,
            editbuf,
            multimode,
            header.patt_start,
        );
    }
    mdb.end_flag = false;
    mdb.speed_cnt = 0;
    mdb.player_enable = true;
}

#[derive(Debug, Copy, Clone)]
pub(crate) struct Cdb {
    macro_run: i8,
    efx_run: i8,
    new_style_macro: u8,
    prev_note: u8,
    curr_note: u8,
    velocity: u8,
    fine_tune: u8,
    key_up: u8,
    really_wait: u8,
    macro_ptr: u32,
    macro_step: u16,
    macro_wait: u16,
    macro_num: u16,
    loop_: i16,
    curr_addr: u32,
    save_addr: u32,
    curr_len: u16,
    save_len: u16,
    wait_dma_count: u16,
    env_reset: u8,
    env_time: u8,
    env_rate: u8,
    env_end_vol: i8,
    cur_vol: i8,
    vib_offset: i16,
    vib_width: i8,
    vib_flag: u8,
    vib_reset: u8,
    vib_time: u8,
    porta_reset: u8,
    porta_time: u8,
    cur_period: u16,
    dest_period: u16,
    porta_per: u16,
    porta_rate: i16,
    add_begin_time: u8,
    add_begin_reset: u8,
    return_ptr: u16,
    return_step: u16,
    add_begin: i32,
    sfx_flag: u8,
    sfx_priority: u8,
    sfx_lock_time: i16,
    sfx_code: u32,
    hw_idx: usize,
}

impl Cdb {
    pub(crate) const fn default() -> Self {
        Self {
            macro_run: 0,
            efx_run: 0,
            new_style_macro: 0,
            prev_note: 0,
            curr_note: 0,
            velocity: 0,
            fine_tune: 0,
            key_up: 0,
            really_wait: 0,
            macro_ptr: 0,
            macro_step: 0,
            macro_wait: 0,
            macro_num: 0,
            loop_: 0,
            curr_addr: 0,
            save_addr: 0,
            curr_len: 0,
            save_len: 0,
            wait_dma_count: 0,
            env_reset: 0,
            env_time: 0,
            env_rate: 0,
            env_end_vol: 0,
            cur_vol: 0,
            vib_offset: 0,
            vib_width: 0,
            vib_flag: 0,
            vib_reset: 0,
            vib_time: 0,
            porta_reset: 0,
            porta_time: 0,
            cur_period: 0,
            dest_period: 0,
            porta_per: 0,
            porta_rate: 0,
            add_begin_time: 0,
            add_begin_reset: 0,
            return_ptr: 0,
            return_step: 0,
            add_begin: 0,
            sfx_flag: 0,
            sfx_priority: 0,
            sfx_lock_time: 0,
            sfx_code: 0,
            hw_idx: 0,
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub(crate) struct Idb {
    cue: [u16; 4usize],
}
impl Idb {
    pub(crate) const fn default() -> Self {
        Self { cue: [0; 4] }
    }
}

#[derive(Debug, Copy, Clone)]
pub(crate) struct Mdb {
    pub(crate) player_enable: bool,
    end_flag: bool,
    curr_song: i8,
    speed_cnt: u16,
    cia_save: u16,
    play_patt_flag: u16,
    master_vol: i8,
    fade_dest: i8,
    fade_time: i8,
    fade_reset: i8,
    fade_slope: i8,
    track_loop: i16,
}
impl Mdb {
    pub(crate) const fn default() -> Self {
        Self {
            player_enable: false,
            end_flag: false,
            curr_song: 0,
            speed_cnt: 0,
            cia_save: 0,
            play_patt_flag: 0,
            master_vol: 0,
            fade_dest: 0,
            fade_time: 0,
            fade_reset: 0,
            fade_slope: 0,
            track_loop: 0,
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub(crate) struct Pdb {
    pub(crate) addr: u32,
    pub(crate) num: u8,
    xpose: i8,
    loop_: u16,
    step: u16,
    wait: u8,
    ro_addr: u16,
    ro_step: u16,
}
impl Pdb {
    const fn default() -> Self {
        Self {
            addr: 0,
            num: 0,
            xpose: 0,
            loop_: 0,
            step: 0,
            wait: 0,
            ro_addr: 0,
            ro_step: 0,
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub(crate) struct Pdblk {
    first_pos: u16,
    last_pos: u16,
    curr_pos: u16,
    prescale: u16,
    pub(crate) p: PdbArr,
}

type PdbArr = [Pdb; MAX_CHANNELS as usize];

impl Pdblk {
    pub(crate) const fn default() -> Self {
        Self {
            first_pos: 0,
            last_pos: 0,
            curr_pos: 0,
            prescale: 0,
            p: [Pdb::default(); MAX_CHANNELS as usize],
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub(crate) struct Hdb {
    pub(crate) pos: u32,
    pub(crate) delta: u32,
    pub(crate) slen: u16,
    pub(crate) sample_len: u16,
    pub(crate) sbeg: usize,
    pub(crate) sample_start: usize,
    pub(crate) vol: u8,
    pub(crate) mode: u8,
    pub(crate) loop_fn: Option<fn(&mut Hdb, &mut CdbArr) -> i32>,
    pub(crate) cdb_idx: Option<usize>,
}
impl Hdb {
    pub(crate) const fn default() -> Self {
        Self {
            pos: 0,
            delta: 0,
            slen: 0,
            sample_len: 0,
            sbeg: 0,
            sample_start: 0,
            vol: 0,
            mode: 0,
            loop_fn: None,
            cdb_idx: None,
        }
    }
}
