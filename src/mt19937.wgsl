// License: Apache 2.0 OR MIT, at your option
// Based on code from https://github.com/dcrewi/rust-mersenne-twister

struct OutputVec {
    size: atomic<u32>,
    data: array<u32>,
}

// Input to the shader. The length of the array is determined by what buffer is bound.
//
// Out of bounds accesses 
@group(0) @binding(0)
var<storage, read> input: array<u32>;
// Output of the shader.  
@group(0) @binding(1)
var<storage, read_write> output: OutputVec;

var<workgroup> output_index: atomic<u32>;

const N: u32 = 624;
const M: u32 = 397;
const MATRIX_A: u32 = 0x9908b0df;
const UPPER_MASK: u32 = 0x80000000;
const LOWER_MASK: u32 = 0x7fffffff;

struct Mersenne {
    idx: u32,
    state: array<u32, N>,
}

fn init() -> Mersenne {
    return Mersenne(0, array<u32, N>());
}

fn reseed(mt: ptr<function, Mersenne>, seed: u32) {
    (*mt).idx = N;
    (*mt).state[0] = seed;
    for (var i: u32 = 1; i < N; i++) {
        (*mt).state[i] = 1812433253 * ((*mt).state[i - 1] ^ ((*mt).state[i - 1] >> 30)) + i;
    }
}

fn temper(y: u32) -> u32 {
    var x = y;
    x ^= x >> 11;
    x ^= (x << 7) & 0x9d2c5680;
    x ^= (x << 15) & 0xefc60000;
    x ^= x >> 18;
    return x;
}

fn fill_next_state(mt: ptr<function, Mersenne>) {
    for (var i: u32 = 0; i < N - M; i++) {
        let x = ((*mt).state[i] & UPPER_MASK) | ((*mt).state[i + 1] & LOWER_MASK);
        (*mt).state[i] = (*mt).state[i + M] ^ (x >> 1) ^ ((x & 1) * MATRIX_A);
    }

    /*
    for (var i: u32 = N - M; i < N - 1; i++) {
        let x = ((*mt).state[i] & UPPER_MASK) | ((*mt).state[i + 1] & LOWER_MASK);
        (*mt).state[i] = (*mt).state[i + M - N] ^ (x >> 1) ^ ((x & 1) * MATRIX_A);
    }

    let x = ((*mt).state[N - 1] & UPPER_MASK) | ((*mt).state[0] & LOWER_MASK);
    (*mt).state[N - 1] = (*mt).state[M - 1] ^ (x >> 1) ^ ((x & 1) * MATRIX_A);
    */

    (*mt).idx = 0;
}

fn next(mt: ptr<function, Mersenne>) -> u32 {
    if (*mt).idx >= N {
        fill_next_state(mt);
    }
    let x = (*mt).state[(*mt).idx];
    (*mt).idx++;
    return temper(x);
}

// Ideal workgroup size depends on the hardware, the workload, and other factors. However, it should
// _generally_ be a multiple of 64. Common sizes are 64x1x1, 256x1x1; or 8x8x1, 16x16x1 for 2D workloads.
@compute @workgroup_size(256, 1, 1)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    // Compute the first value and write to the output.

    // step is the step X/256
    // as the shader is executed 256 times
    // and each time computes 2^24 seeds
    let step = input[0];

    var mt = init();
    let seed = global_id.x * 256 + step;
    reseed(&mt, seed);

    var seed_is_valid = true;
    let end = arrayLength(&input);
    for (var i: u32 = 1; i < end; i += 4) {
        let match_min = input[i + 0];
        let match_max = input[i + 1];
        let range_min = input[i + 2];
        let range_max = input[i + 3];

        let nextint = next(&mt);
        let randint = select(
            nextint % (range_max - range_min + 1) + range_min,
            nextint >> 1,
            range_min == 0 && range_max == 0x7fffffff
        );
        
        if randint < match_min || randint > match_max {
            seed_is_valid = false;
            break;
        }
    }

    if seed_is_valid {
        let insert_index: u32 = atomicAdd(&output.size, 1);
        if insert_index < arrayLength(&output.data) {
            output.data[insert_index] = seed;
        }
    }
}
