// License: Apache 2.0 OR MIT, at your option
// Based on code from https://github.com/dcrewi/rust-mersenne-twister

// Input to the shader. The length of the array is determined by what buffer is bound.
//
// Out of bounds accesses 
@group(0) @binding(0)
var<storage, read> input: array<u32>;
// Output of the shader.  
@group(0) @binding(1)
var<storage, read_write> output: array<u32>;

const N: u32 = 624;
const M: u32 = 397;
const MATRIX_A: u32 = 0x9908b0df;
const UPPER_MASK: u32 = 0x80000000;
const LOWER_MASK: u32 = 0x7fffffff;

struct Mersenne {
	idx: u32,
	state: array<u32, N>
}

fn init() -> Mersenne {
	var mt = Mersenne(0, array<u32, N>());
	for (var i: u32 = 0; i < N; i++) {
		mt.state[i] = 0;
	}
	return mt;
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
        for (var i: u32 = N - M; i < N - 1; i++) {
            let x = ((*mt).state[i] & UPPER_MASK) | ((*mt).state[i + 1] & LOWER_MASK);
            (*mt).state[i] = (*mt).state[i + M - N] ^ (x >> 1) ^ ((x & 1) * MATRIX_A);
        }
        let x = ((*mt).state[N - 1] & UPPER_MASK) | ((*mt).state[0] & LOWER_MASK);
        (*mt).state[N - 1] = (*mt).state[M - 1] ^ (x >> 1) ^ ((x & 1) * MATRIX_A);
        (*mt).idx = 0;
}

fn next(mt: ptr<function, Mersenne>) -> u32 {
	if ((*mt).idx >= N) {
		fill_next_state(mt);
	}
	let x = (*mt).state[(*mt).idx];
	(*mt).idx++;
	return temper(x);
}

// Ideal workgroup size depends on the hardware, the workload, and other factors. However, it should
// _generally_ be a multiple of 64. Common sizes are 64x1x1, 256x1x1; or 8x8x1, 16x16x1 for 2D workloads.
@compute @workgroup_size(256)
fn mersenne(@builtin(global_invocation_id) global_id: vec3<u32>) {
    // While compute invocations are 3d, we're only using one dimension.
    let index = global_id.x;

    // Because we're using a workgroup size of 64, if the input size isn't a multiple of 64,
    // we will have some "extra" invocations. This is fine, but we should tell them to stop
    // to avoid out-of-bounds accesses.
    let array_length = arrayLength(&input);
    if (global_id.x >= array_length) {
        return;
    }

    // Compute the first value and write to the output.
    var mt = init();
    let seed = input[global_id.x];
    reseed(&mt, seed);
    let result = next(&mt);
    output[global_id.x] = result;
}
