//! Voice activity detection for subtitle sync by ear.
//!
//! A port of WebRTC's `common_audio/vad` (most aggressive mode, 16 kHz mono 16-bit input) as used
//! by subtitle sync tools. Fixed-point arithmetic is kept so the decisions match the original.
//!
//! Copyright (c) 2011, The WebRTC project authors. All rights reserved.
//! Redistribution and use in source and binary forms, with or without modification, are permitted
//! provided that the following conditions are met:
//! * Redistributions of source code must retain the above copyright notice, this list of
//!   conditions and the following disclaimer.
//! * Redistributions in binary form must reproduce the above copyright notice, this list of
//!   conditions and the following disclaimer in the documentation and/or other materials provided
//!   with the distribution.
//! * Neither the name of Google nor the names of its contributors may be used to endorse or
//!   promote products derived from this software without specific prior written permission.
//! THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR
//! IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY
//! AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR
//! CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
//! DAMAGES HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR
//! TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN
//! IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

const SPECTRUM_WEIGHT: [i32; 6] = [6, 8, 10, 12, 14, 16];
const NOISE_UPDATE_CONST: i32 = 655;
const SPEECH_UPDATE_CONST: i32 = 6554;
const BACK_ETA: i32 = 154;
const MINIMUM_DIFFERENCE: [i32; 6] = [544, 544, 576, 576, 576, 576];
const MAXIMUM_SPEECH: [i32; 6] = [11392, 11392, 11520, 11520, 11520, 11520];
const MINIMUM_MEAN: [i32; 2] = [640, 768];
const MAXIMUM_NOISE: [i32; 6] = [9216, 9088, 8960, 8832, 8704, 8576];
const NOISE_DATA_WEIGHTS: [i32; 12] = [34, 62, 72, 66, 53, 25, 94, 66, 56, 62, 75, 103];
const SPEECH_DATA_WEIGHTS: [i32; 12] = [48, 82, 45, 87, 50, 47, 80, 46, 83, 41, 78, 81];
const NOISE_DATA_MEANS: [i32; 12] = [6738, 4892, 7065, 6715, 6771, 3369, 7646, 3863, 7820, 7266, 5020, 4362];
const SPEECH_DATA_MEANS: [i32; 12] = [8306, 10085, 10078, 11823, 11843, 6309, 9473, 9571, 10879, 7581, 8180, 7483];
const NOISE_DATA_STDS: [i32; 12] = [378, 1064, 493, 582, 688, 593, 474, 697, 475, 688, 421, 455];
const SPEECH_DATA_STDS: [i32; 12] = [555, 505, 567, 524, 585, 1231, 509, 828, 492, 1540, 1079, 850];
const MAX_SPEECH_FRAMES: i32 = 6;
const MIN_STD: i32 = 384;
const MIN_ENERGY: i32 = 10;
const OVER_HANG_MAX_1: i32 = 6;
const OVER_HANG_MAX_2: i32 = 9;
const INDIVIDUAL: i32 = 94;
const TOTAL: i32 = 1100;
const LOG_CONST: i32 = 24660;
const LOG_ENERGY_INT_PART: i32 = 14336;
const HP_ZERO_COEFS: [i32; 3] = [6631, -13262, 6631];
const HP_POLE_COEFS: [i32; 3] = [0, -7756, 5620];
const ALL_PASS_COEFS_Q15: [i32; 2] = [20972, 5571];
const OFFSET_VECTOR: [i32; 6] = [368, 368, 272, 176, 176, 176];
const ALL_PASS_COEFS_Q13: [i32; 2] = [5243, 1392];
const COMP_VAR: i32 = 22005;
const LOG2_EXP: i32 = 5909;

#[inline]
fn s16<T: Into<i64>>(x: T) -> i16 {
    (x.into() as u16) as i16
}
#[inline]
fn s32<T: Into<i64>>(x: T) -> i32 {
    x.into() as i32
}
#[inline]
fn clz32(x: u32) -> u32 {
    x.leading_zeros()
}
#[inline]
fn norm_w32(a: i32) -> u32 {
    if a == 0 {
        0
    } else {
        clz32(if a < 0 { !a as u32 } else { a as u32 }) - 1
    }
}
#[inline]
fn norm_u32(a: u32) -> u32 {
    clz32(a)
}
#[inline]
fn size_in_bits(n: u32) -> u32 {
    32 - clz32(n)
}
#[inline]
fn div_w32_w16(num: i32, den: i32) -> i32 {
    if den != 0 {
        s32(num as i64 / den as i64)
    } else {
        i32::MAX
    }
}

/// Running state of one detector: feed a film's sound in order, 10 ms at a time.
pub struct VoiceDetector {
    down: [i32; 2],
    noise_means: [i32; 12],
    speech_means: [i32; 12],
    noise_stds: [i32; 12],
    speech_stds: [i32; 12],
    frame_counter: i32,
    over_hang: i32,
    num_of_speech: i32,
    index_vector: [i32; 96],
    low_value_vector: [i32; 96],
    mean_value: [i32; 6],
    upper_state: [i32; 5],
    lower_state: [i32; 5],
    hp_filter_state: [i32; 4],
    nb: [i16; 80],
    hp120: [i16; 40],
    lp120: [i16; 40],
    hp60: [i16; 20],
    lp60: [i16; 20],
    features: [i32; 6],
    delta_n: [i32; 12],
    delta_s: [i32; 12],
    ngprvec: [i32; 12],
    sgprvec: [i32; 12],
}

impl Default for VoiceDetector {
    fn default() -> Self {
        Self {
            down: [0; 2],
            noise_means: NOISE_DATA_MEANS,
            speech_means: SPEECH_DATA_MEANS,
            noise_stds: NOISE_DATA_STDS,
            speech_stds: SPEECH_DATA_STDS,
            frame_counter: 0,
            over_hang: 0,
            num_of_speech: 0,
            index_vector: [0; 96],
            low_value_vector: [10000; 96],
            mean_value: [1600; 6],
            upper_state: [0; 5],
            lower_state: [0; 5],
            hp_filter_state: [0; 4],
            nb: [0; 80],
            hp120: [0; 40],
            lp120: [0; 40],
            hp60: [0; 20],
            lp60: [0; 20],
            features: [0; 6],
            delta_n: [0; 12],
            delta_s: [0; 12],
            ngprvec: [0; 12],
            sgprvec: [0; 12],
        }
    }
}

impl VoiceDetector {
    /// Whether someone speaks in the 160 samples of `pcm` from `at`.
    pub fn frame(&mut self, pcm: &[i16], at: usize) -> bool {
        let mut t1 = self.down[0];
        let mut t2 = self.down[1];
        for n in 0..80 {
            let a = pcm[at + 2 * n] as i32;
            let b = pcm[at + 2 * n + 1] as i32;
            let o1 = s16((t1 >> 1) + ((ALL_PASS_COEFS_Q13[0] * a) >> 14));
            t1 = s32(a as i64 - ((ALL_PASS_COEFS_Q13[0] * o1 as i32) >> 12) as i64);
            let o2 = s16((t2 >> 1) + ((ALL_PASS_COEFS_Q13[1] * b) >> 14));
            t2 = s32(b as i64 - ((ALL_PASS_COEFS_Q13[1] * o2 as i32) >> 12) as i64);
            self.nb[n] = s16(o1 as i32 + o2 as i32);
        }
        self.down = [t1, t2];
        let power = self.calculate_features();
        self.gmm_probability(power) != 0
    }

    fn calculate_features(&mut self) -> i32 {
        let mut total = 0i32;
        self.split_filter(0, 80, 0);
        let hp_a = self.hp120;
        let lp_a = self.lp120;
        let band = 1;
        self.split_filter_data(&hp_a, 0, 40, band);
        self.features[5] = log_of_energy(&self.hp60, 20, OFFSET_VECTOR[5], &mut total);
        self.features[4] = log_of_energy(&self.lp60, 20, OFFSET_VECTOR[4], &mut total);
        self.split_filter_data(&lp_a, 0, 40, 2);
        self.features[3] = log_of_energy(&self.hp60, 20, OFFSET_VECTOR[3], &mut total);
        let lp_b = self.lp60;
        self.split_filter_data(&lp_b, 0, 20, 3);
        self.features[2] = log_of_energy(&self.hp120, 10, OFFSET_VECTOR[2], &mut total);
        let lp_c = self.lp120;
        self.split_filter_data(&lp_c, 0, 10, 4);
        self.features[1] = log_of_energy(&self.hp60, 5, OFFSET_VECTOR[1], &mut total);
        self.high_pass_filter(5);
        self.features[0] = log_of_energy(&self.hp120, 5, OFFSET_VECTOR[0], &mut total);
        total
    }

    /// Splits `self.nb` into `hp120`/`lp120` (band 0) at 2000 Hz.
    fn split_filter(&mut self, _at: usize, length: usize, _band: usize) {
        let data = self.nb;
        self.upper_state[0] = Self::all_pass_filter(&data, 0, length / 2, ALL_PASS_COEFS_Q15[0], self.upper_state[0], &mut self.hp120);
        self.lower_state[0] = Self::all_pass_filter(&data, 1, length / 2, ALL_PASS_COEFS_Q15[1], self.lower_state[0], &mut self.lp120);
        for i in 0..length / 2 {
            let tmp = self.hp120[i];
            self.hp120[i] = s16(self.hp120[i] as i32 - self.lp120[i] as i32);
            self.lp120[i] = s16(self.lp120[i] as i32 + tmp as i32);
        }
    }

    fn split_filter_data(&mut self, data: &[i16], at: usize, length: usize, band: usize) {
        let half = length >> 1;
        self.upper_state[band] = Self::all_pass_filter(data, at, half, ALL_PASS_COEFS_Q15[0], self.upper_state[band], &mut self.hp120);
        self.lower_state[band] = Self::all_pass_filter(data, at + 1, half, ALL_PASS_COEFS_Q15[1], self.lower_state[band], &mut self.lp120);
        for i in 0..half {
            let tmp = self.hp120[i];
            self.hp120[i] = s16(self.hp120[i] as i32 - self.lp120[i] as i32);
            self.lp120[i] = s16(self.lp120[i] as i32 + tmp as i32);
        }
    }

    fn all_pass_filter(data: &[i16], at: usize, n: usize, coef: i32, state: i32, out: &mut [i16]) -> i32 {
        let mut state32 = s32(state as i64 * 65536);
        let mut p = at;
        for i in 0..n {
            let x = data[p] as i32;
            let tmp32 = s32(state32 as i64 + coef as i64 * x as i64);
            let tmp16 = s16(tmp32 >> 16);
            out[i] = tmp16;
            state32 = s32(s32(x as i64 * 16384 - coef as i64 * tmp16 as i64) as i64 * 2);
            p += 2;
        }
        s16(state32 >> 16) as i32
    }

    fn high_pass_filter(&mut self, n: usize) {
        let fs = &mut self.hp_filter_state;
        for i in 0..n {
            let d = self.lp60[i] as i32;
            let mut tmp32 = s32(HP_ZERO_COEFS[0] as i64 * d as i64 + HP_ZERO_COEFS[1] as i64 * fs[0] as i64 + HP_ZERO_COEFS[2] as i64 * fs[1] as i64);
            fs[1] = fs[0];
            fs[0] = d;
            tmp32 = s32(tmp32 as i64 - HP_POLE_COEFS[1] as i64 * fs[2] as i64);
            tmp32 = s32(tmp32 as i64 - HP_POLE_COEFS[2] as i64 * fs[3] as i64);
            fs[3] = fs[2];
            fs[2] = s16(tmp32 >> 14) as i32;
            self.hp120[i] = s16(fs[2]);
        }
    }

    fn gaussian(input: i32, mean: i32, std: i32, delta: &mut [i32], i: usize) -> i32 {
        let inv_std = s16(div_w32_w16(131072 + (std >> 1), std) as i64) as i32;
        let mut tmp16 = s16(inv_std >> 2) as i32;
        let inv_std2 = s16((tmp16 as i64 * tmp16 as i64) >> 2) as i32;
        tmp16 = s16((input as i64) << 3) as i32;
        tmp16 = s16(tmp16 as i64 - mean as i64) as i32;
        let d = s16((inv_std2 as i64 * tmp16 as i64) >> 10) as i32;
        delta[i] = d;
        let tmp32 = s32((d as i64 * tmp16 as i64) >> 9);
        let mut exp_value = 0i32;
        if tmp32 < COMP_VAR {
            let mut t = s16((LOG2_EXP as i64 * tmp32 as i64) >> 12) as i32;
            t = s16(-(t as i64)) as i32;
            exp_value = 0x0400 | (t & 0x03FF);
            t = s16((t as i64) ^ 0xFFFF) as i32;
            t >>= 10;
            t += 1;
            exp_value = s16((exp_value as i64) >> t) as i32;
        }
        s32(inv_std as i64 * exp_value as i64)
    }

    fn weighted_average(data: &mut [i32], ch: usize, offset: i32, weights: &[i32; 12]) -> i32 {
        let mut sum = 0i64;
        for k in 0..2 {
            let g = ch + k * 6;
            data[g] = s16(data[g] as i64 + offset as i64) as i32;
            sum += data[g] as i64 * weights[g] as i64;
        }
        s32(sum)
    }

    fn find_minimum(&mut self, feature: i32, ch: usize) -> i32 {
        let offset = ch << 4;
        let age = &mut self.index_vector;
        let smallest = &mut self.low_value_vector;
        for i in 0..16 {
            if age[offset + i] != 100 {
                age[offset + i] += 1;
            } else {
                for j in i..15 {
                    smallest[offset + j] = smallest[offset + j + 1];
                    age[offset + j] = age[offset + j + 1];
                }
                age[offset + 15] = 101;
                smallest[offset + 15] = 10000;
            }
        }
        let mut position = None;
        for i in 0..16 {
            if feature < smallest[offset + i] {
                position = Some(i);
                break;
            }
        }
        if let Some(position) = position {
            for i in (position + 1..16).rev() {
                smallest[offset + i] = smallest[offset + i - 1];
                age[offset + i] = age[offset + i - 1];
            }
            smallest[offset + position] = feature;
            age[offset + position] = 1;
        }
        let mut median = 1600;
        if self.frame_counter > 2 {
            median = smallest[offset + 2];
        } else if self.frame_counter > 0 {
            median = smallest[offset];
        }
        let alpha = if self.frame_counter > 0 {
            if median < self.mean_value[ch] {
                // smoothingDown: 0.2 in Q15
                6553
            } else {
                // smoothingUp: 0.99 in Q15
                32439
            }
        } else {
            0
        };
        let tmp32 = (alpha + 1) as i64 * self.mean_value[ch] as i64 + (32767 - alpha) as i64 * median as i64 + 16384;
        self.mean_value[ch] = s16(tmp32 >> 15) as i32;
        self.mean_value[ch]
    }

    fn gmm_probability(&mut self, total_power: i32) -> i32 {
        let mut vadflag = 0i32;
        if total_power > MIN_ENERGY {
            self.ngprvec.fill(0);
            self.sgprvec.fill(0);
            let mut sum_llr = 0i32;
            for ch in 0..6 {
                let mut h0_test = 0i64;
                let mut h1_test = 0i64;
                let mut noise_prob = [0i32; 2];
                let mut speech_prob = [0i32; 2];
                for k in 0..2 {
                    let g = ch + k * 6;
                    noise_prob[k] = s32(NOISE_DATA_WEIGHTS[g] as i64 * Self::gaussian(self.features[ch], NOISE_DATA_MEANS[g], NOISE_DATA_STDS[g], &mut self.delta_n, g) as i64);
                    h0_test += noise_prob[k] as i64;
                    speech_prob[k] = s32(SPEECH_DATA_WEIGHTS[g] as i64 * Self::gaussian(self.features[ch], SPEECH_DATA_MEANS[g], SPEECH_DATA_STDS[g], &mut self.delta_s, g) as i64);
                    h1_test += speech_prob[k] as i64;
                }
                let h0 = h0_test as i32;
                let h1 = h1_test as i32;
                let shifts_h0 = if h0 == 0 { 31 } else { norm_w32(h0) };
                let shifts_h1 = if h1 == 0 { 31 } else { norm_w32(h1) };
                let llr = s16(shifts_h0 as i64 - shifts_h1 as i64) as i32;
                sum_llr = s32(sum_llr as i64 + llr as i64 * SPECTRUM_WEIGHT[ch] as i64);
                if llr * 4 > INDIVIDUAL {
                    vadflag = 1;
                }
                let h0s = s16((h0 as i64) >> 12) as i32;
                if h0s > 0 {
                    let tmp = s32(((noise_prob[0] as u32 & 0xFFFF_F000) as i64) << 2);
                    self.ngprvec[ch] = s16(div_w32_w16(tmp, h0s) as i64) as i32;
                    self.ngprvec[ch + 6] = s16(16384 - self.ngprvec[ch] as i64) as i32;
                } else {
                    self.ngprvec[ch] = 16384;
                }
                let h1s = s16((h1 as i64) >> 12) as i32;
                if h1s > 0 {
                    let tmp = s32(((speech_prob[0] as u32 & 0xFFFF_F000) as i64) << 2);
                    self.sgprvec[ch] = s16(div_w32_w16(tmp, h1s) as i64) as i32;
                    self.sgprvec[ch + 6] = s16(16384 - self.sgprvec[ch] as i64) as i32;
                } else {
                    self.sgprvec[ch] = 16384;
                }
            }
            if sum_llr >= TOTAL {
                vadflag |= 1;
            }

            let mut maxspe = 12800i32;
            for ch in 0..6 {
                let feature_minimum = self.find_minimum(self.features[ch], ch);
                let mut noise_global_mean = Self::weighted_average(&mut self.noise_means, ch, 0, &NOISE_DATA_WEIGHTS);
                let tmp1s16 = s16((noise_global_mean as i64) >> 6) as i32;
                for k in 0..2 {
                    let g = ch + k * 6;
                    let nmk = self.noise_means[g];
                    let smk = self.speech_means[g];
                    let mut nsk = self.noise_stds[g];
                    let mut ssk = self.speech_stds[g];
                    let mut nmk2 = nmk;
                    if vadflag == 0 {
                        let delt = s16((self.ngprvec[g] as i64 * self.delta_n[g] as i64) >> 11) as i32;
                        nmk2 = s16(nmk as i64 + ((delt as i64 * NOISE_UPDATE_CONST as i64) >> 22)) as i32;
                    }
                    let ndelt = s16((feature_minimum as i64) << 4) as i32 - tmp1s16;
                    let mut nmk3 = s16(nmk2 as i64 + ((ndelt as i64 * BACK_ETA as i64) >> 9)) as i32;
                    let lowest = s16((((k + 5) << 7)) as i64) as i32;
                    let highest = s16((((72 + k) - ch) << 7) as i64) as i32;
                    if nmk3 < lowest {
                        nmk3 = lowest;
                    }
                    if nmk3 > highest {
                        nmk3 = highest;
                    }
                    self.noise_means[g] = nmk3;
                    if vadflag != 0 {
                        let delt = s16((self.sgprvec[g] as i64 * self.delta_s[g] as i64) >> 11) as i32;
                        let tmp = s16((delt as i64 * SPEECH_UPDATE_CONST as i64) >> 21) as i32;
                        let mut smk2 = s16(smk as i64 + ((tmp as i64 + 1) >> 1)) as i32;
                        let maxmu = s16(maxspe as i64 + 640) as i32;
                        if smk2 < MINIMUM_MEAN[k] {
                            smk2 = MINIMUM_MEAN[k];
                        }
                        if smk2 > maxmu {
                            smk2 = maxmu;
                        }
                        self.speech_means[g] = smk2;
                        let mut tmp = s16((smk as i64 + 4) >> 3) as i32;
                        tmp = s16(self.features[ch] as i64 - tmp as i64) as i32;
                        let t1 = s32((self.delta_s[g] as i64 * tmp as i64) >> 3);
                        let t2 = t1 - 4096;
                        tmp = self.sgprvec[g] >> 2;
                        let t1 = s32(tmp as i64 * t2 as i64);
                        let t2 = t1 >> 4;
                        let den = s16(ssk as i64 * 10) as i32;
                        tmp = if t2 > 0 {
                            s16(div_w32_w16(t2, den) as i64) as i32
                        } else {
                            s16(-(div_w32_w16(-t2, den) as i64)) as i32
                        };
                        tmp = s16(tmp as i64 + 128) as i32;
                        ssk = s16(ssk as i64 + (tmp as i64 >> 8)) as i32;
                        if ssk < MIN_STD {
                            ssk = MIN_STD;
                        }
                        self.speech_stds[g] = ssk;
                    } else {
                        let mut tmp = s16(self.features[ch] as i64 - (nmk as i64 >> 3)) as i32;
                        let mut t1 = s32((self.delta_n[g] as i64 * tmp as i64) >> 3);
                        t1 -= 4096;
                        tmp = (self.ngprvec[g] + 2) >> 2;
                        let t2 = s32(tmp as i64 * t1 as i64);
                        t1 = t2 >> 14;
                        tmp = if t1 > 0 {
                            s16(div_w32_w16(t1, nsk) as i64) as i32
                        } else {
                            s16(-(div_w32_w16(-t1, nsk) as i64)) as i32
                        };
                        tmp = s16(tmp as i64 + 32) as i32;
                        nsk = s16(nsk as i64 + (tmp as i64 >> 6)) as i32;
                        if nsk < MIN_STD {
                            nsk = MIN_STD;
                        }
                        self.noise_stds[g] = nsk;
                    }
                }
                noise_global_mean = Self::weighted_average(&mut self.noise_means, ch, 0, &NOISE_DATA_WEIGHTS);
                let mut speech_global_mean = Self::weighted_average(&mut self.speech_means, ch, 0, &SPEECH_DATA_WEIGHTS);
                let diff = s16(s16(noise_global_mean as i64 >> 9) as i64) as i32 - s16(s16(speech_global_mean as i64 >> 9) as i64) as i32;
                // Dart computes speech - noise; keep that sign.
                let diff = -diff;
                if diff < MINIMUM_DIFFERENCE[ch] {
                    let tmp = MINIMUM_DIFFERENCE[ch] - diff;
                    let up = s16((13 * tmp as i64) >> 2) as i32;
                    let down = s16((3 * tmp as i64) >> 2) as i32;
                    speech_global_mean = Self::weighted_average(&mut self.speech_means, ch, up, &SPEECH_DATA_WEIGHTS);
                    noise_global_mean = Self::weighted_average(&mut self.noise_means, ch, -down, &NOISE_DATA_WEIGHTS);
                }
                maxspe = MAXIMUM_SPEECH[ch];
                let mut t2 = s16((speech_global_mean as i64) >> 7) as i32;
                if t2 > maxspe {
                    t2 = s16(t2 as i64 - maxspe as i64) as i32;
                    for k in 0..2 {
                        self.speech_means[ch + k * 6] = s16(self.speech_means[ch + k * 6] as i64 - t2 as i64) as i32;
                    }
                }
                t2 = s16((noise_global_mean as i64) >> 7) as i32;
                if t2 > MAXIMUM_NOISE[ch] {
                    t2 = s16(t2 as i64 - MAXIMUM_NOISE[ch] as i64) as i32;
                    for k in 0..2 {
                        self.noise_means[ch + k * 6] = s16(self.noise_means[ch + k * 6] as i64 - t2 as i64) as i32;
                    }
                }
            }
            self.frame_counter += 1;
        }
        if vadflag == 0 {
            if self.over_hang > 0 {
                vadflag = 2 + self.over_hang;
                self.over_hang -= 1;
            }
            self.num_of_speech = 0;
        } else {
            self.num_of_speech += 1;
            if self.num_of_speech > MAX_SPEECH_FRAMES {
                self.num_of_speech = MAX_SPEECH_FRAMES;
                self.over_hang = OVER_HANG_MAX_2;
            } else {
                self.over_hang = OVER_HANG_MAX_1;
            }
        }
        vadflag
    }
}

fn log_of_energy(data: &[i16], n: usize, offset: i32, total: &mut i32) -> i32 {
    let nbits = size_in_bits(n as u32);
    let mut smax = -1i32;
    for i in 0..n {
        let a = data[i] as i32;
        let sabs = s16(if a > 0 { a } else { -a }) as i32;
        if sabs > smax {
            smax = sabs;
        }
    }
    let t = norm_w32(s32(smax as i64 * smax as i64));
    let scaling = if smax == 0 { 0 } else if t > nbits { 0 } else { nbits - t };
    let mut en = 0i64;
    for i in 0..n {
        en += ((data[i] as i64 * data[i] as i64) >> scaling) as i64;
    }
    let mut tot_rshifts: i32 = scaling as i32;
    let mut energy = en as u32;
    if energy == 0 {
        return offset;
    }
    let normalizing: i32 = 17 - norm_u32(energy) as i32;
    tot_rshifts += normalizing;
    energy = if normalizing < 0 {
        energy << (-normalizing) as u32
    } else {
        energy >> normalizing as u32
    };
    let mut log2_energy = LOG_ENERGY_INT_PART;
    log2_energy = s16(log2_energy as i64 + ((energy & 0x3FFF) >> 4) as i64) as i32;
    let mut log_energy = s16((LOG_CONST as i64 * log2_energy as i64) >> 19) as i32;
    log_energy = s16(log_energy as i64 + ((tot_rshifts as i64 * LOG_CONST as i64) >> 9)) as i32;
    if log_energy < 0 {
        log_energy = 0;
    }
    log_energy = s16(log_energy as i64 + offset as i64) as i32;
    if *total <= MIN_ENERGY {
        *total = if tot_rshifts >= 0 {
            s16(*total as i64 + MIN_ENERGY as i64 + 1) as i32
        } else {
            s16(*total as i64 + s16((energy as i64) >> (-tot_rshifts) as u32) as i64) as i32
        };
    }
    log_energy
}

/// Speech in `pcm` (16 kHz mono, 16-bit), one byte per 10 ms: 1 where someone speaks.
pub fn detect_speech(pcm: &[i16]) -> Vec<u8> {
    let mut vad = VoiceDetector::default();
    let mut out = Vec::with_capacity(pcm.len() / 160);
    for at in (0..pcm.len()).step_by(160) {
        if at + 160 > pcm.len() {
            break;
        }
        out.push(u8::from(vad.frame(pcm, at)));
    }
    out
}
