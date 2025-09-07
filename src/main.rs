use std::{io, io::Write, num::NonZeroU64, str::FromStr};
use wgpu::util::DeviceExt;

fn print_usage() {
    println!(
        "Usage: php_mt_seed.rs VALUE_OR_MATCH_MIN [MATCH_MAX [RANGE_MIN RANGE_MAX]] ...\n\n\
         This tool is similar to openwall's php_mt_seed, though php_mt_seed.rs only supports PHP 7.1.0+\n\
         Have a look at openwall's php_mt_seed documentation for more information on CLI arguments:\n\
         - https://www.openwall.com/php_mt_seed/README\n\
         - https://github.com/openwall/php_mt_seed"
    );
}

fn get_arguments() -> Vec<u32> {
    return std::env::args()
        .skip(1) // skip the name of the program
        .map(|s| {
            u32::from_str(&s)
                .unwrap_or_else(|_| panic!("Cannot parse argument {s:?} as an integer."))
        })
        .collect();
}

fn normalize_arguments(arguments: &mut Vec<u32>) {
    let mut len = arguments.len();
    if len % 4 == 1 {
        arguments.push(arguments[len - 1]);
    }

    len = arguments.len();
    if len % 4 == 2 {
        arguments[len - 2] = arguments[len - 2];
        arguments[len - 1] = arguments[len - 1];
        arguments.push(0);
        arguments.push(0x7fffffff);
    }
}

fn lint_arguments(arguments: &Vec<u32>) -> bool {
    if arguments.is_empty() {
        return false;
    }

    for chunk in arguments.chunks(4) {
        match chunk {
            &[match_min, match_max, range_min, range_max] => {
                if match_min > match_max
                    || range_min > range_max
                    || match_max < range_min
                    || match_min > range_max
                    || range_max > 0x7fffffff
                    || match_max > 0x7fffffff
                {
                    return false;
                }
            }
            _ => return false, // if the normalized argument number isn't a multiple of 4
        }
    }

    return true;
}

/// uses the GPU to find the seed given the `arguments` in openwall's php_mt_rand format, and `step` which is between 0 and 256
fn find_mersenne_seed(arguments: &[u32], step: u32) -> Option<Vec<u32>> {
    assert!(step < 256);

    // We first initialize an wgpu `Instance`, which contains any "global" state wgpu needs.
    //
    // This is what loads the vulkan/dx12/metal/opengl libraries.
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());

    // We then create an `Adapter` which represents a physical gpu in the system. It allows
    // us to query information about it and create a `Device` from it.
    //
    // This function is asynchronous in WebGPU, so request_adapter returns a future. On native/webgl
    // the future resolves immediately, so we can block on it without harm.
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("Failed to create adapter");

    // Print out some basic information about the adapter.
    if step == 0 {
        println!("\rRunning on Adapter: {:#?}", adapter.get_info());
    }

    // Check to see if the adapter supports compute shaders. While WebGPU guarantees support for
    // compute shaders, wgpu supports a wider range of devices through the use of "downlevel" devices.
    let downlevel_capabilities = adapter.get_downlevel_capabilities();
    if !downlevel_capabilities
        .flags
        .contains(wgpu::DownlevelFlags::COMPUTE_SHADERS)
    {
        panic!("Adapter does not support compute shaders");
    }

    // We then create a `Device` and a `Queue` from the `Adapter`.
    //
    // The `Device` is used to create and manage GPU resources.
    // The `Queue` is a queue used to submit work for the GPU to process.
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: None,
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::downlevel_defaults(),
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        trace: wgpu::Trace::Off,
    }))
    .expect("Failed to create device");

    // Create a shader module from our shader code. This will parse and validate the shader.
    //
    // `include_wgsl` is a macro provided by wgpu like `include_str` which constructs a ShaderModuleDescriptor.
    // If you want to load shaders differently, you can construct the ShaderModuleDescriptor manually.
    let module = device.create_shader_module(wgpu::include_wgsl!("mt19937.wgsl"));

    let mut input_data = Vec::new();
    input_data.push(step);
    input_data.extend_from_slice(arguments);

    // Create a buffer with the data we want to process on the GPU.
    //
    // `create_buffer_init` is a utility provided by `wgpu::util::DeviceExt` which simplifies creating
    // a buffer with some initial data.
    //
    // We use the `bytemuck` crate to cast the slice of f32 to a &[u8] to be uploaded to the GPU.
    let input_data_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&input_data),
        usage: wgpu::BufferUsages::STORAGE,
    });

    // Now we create a buffer to store the output data.
    let max_results = 1_000;
    let output_buffer_size = max_results * std::mem::size_of::<u32>() as u64;
    let output_data_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: output_buffer_size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });

    // Finally we create a buffer which can be read by the CPU. This buffer is how we will read
    // the data. We need to use a separate buffer because we need to have a usage of `MAP_READ`,
    // and that usage can only be used with `COPY_DST`.
    let download_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: output_buffer_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    // A bind group layout describes the types of resources that a bind group can contain. Think
    // of this like a C-style header declaration, ensuring both the pipeline and bind group agree
    // on the types of resources.
    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[
            // Input buffer
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    // This is the size of a single element in the buffer.
                    min_binding_size: Some(NonZeroU64::new(4).unwrap()),
                    has_dynamic_offset: false,
                },
                count: None,
            },
            // Output buffer
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    // This is the size of a single element in the buffer.
                    min_binding_size: Some(NonZeroU64::new(8).unwrap()),
                    has_dynamic_offset: false,
                },
                count: None,
            },
        ],
    });

    // The bind group contains the actual resources to bind to the pipeline.
    //
    // Even when the buffers are individually dropped, wgpu will keep the bind group and buffers
    // alive until the bind group itself is dropped.
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: input_data_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: output_data_buffer.as_entire_binding(),
            },
        ],
    });

    // The pipeline layout describes the bind groups that a pipeline expects
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[&bind_group_layout],
        push_constant_ranges: &[],
    });

    // The pipeline is the ready-to-go program state for the GPU. It contains the shader modules,
    // the interfaces (bind group layouts) and the shader entry point.
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: Some(&pipeline_layout),
        module: &module,
        entry_point: Some("main"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });

    // The command encoder allows us to record commands that we will later submit to the GPU.
    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

    // A compute pass is a single series of compute operations. While we are recording a compute
    // pass, we cannot record to the encoder.
    let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: None,
        timestamp_writes: None,
    });

    // Set the pipeline that we want to use
    compute_pass.set_pipeline(&pipeline);
    // Set the bind group that we want to use
    compute_pass.set_bind_group(0, &bind_group, &[]);

    // Now we dispatch a series of workgroups. Each workgroup is a 3D grid of individual programs.
    compute_pass.dispatch_workgroups(65535, 1, 1);

    // Now we drop the compute pass, giving us access to the encoder again.
    drop(compute_pass);

    // We add a copy operation to the encoder. This will copy the data from the output buffer on the
    // GPU to the download buffer on the CPU.
    encoder.copy_buffer_to_buffer(
        &output_data_buffer,
        0,
        &download_buffer,
        0,
        output_data_buffer.size(),
    );

    // We finish the encoder, giving us a fully recorded command buffer.
    let command_buffer = encoder.finish();

    // At this point nothing has actually been executed on the gpu. We have recorded a series of
    // commands that we want to execute, but they haven't been sent to the gpu yet.
    //
    // Submitting to the queue sends the command buffer to the gpu. The gpu will then execute the
    // commands in the command buffer in order.
    queue.submit([command_buffer]);

    // We now map the download buffer so we can read it. Mapping tells wgpu that we want to read/write
    // to the buffer directly by the CPU and it should not permit any more GPU operations on the buffer.
    //
    // Mapping requires that the GPU be finished using the buffer before it resolves, so mapping has a callback
    // to tell you when the mapping is complete.
    let buffer_slice = download_buffer.slice(..);
    buffer_slice.map_async(wgpu::MapMode::Read, |_| {
        // In this case we know exactly when the mapping will be finished,
        // so we don't need to do anything in the callback.
    });

    // Wait for the GPU to finish working on the submitted work. This doesn't work on WebGPU, so we would need
    // to rely on the callback to know when the buffer is mapped.
    device.poll(wgpu::PollType::Wait).unwrap();

    // We can now read the data from the buffer.
    let data = buffer_slice.get_mapped_range();
    // Convert the data back to a slice of f32.
    let result: &[u32] = bytemuck::cast_slice(&data);

    // `result` is actually a length + the data + some trailing trash
    // for example for 2 results we might have:
    // [2, 8, 6, 4, 1, 0, 0, 0]
    // here the length is 2
    // and the actual data is [8, 6]
    let subslice_start = 1;
    let subslice_end = 1 + result[0] as usize;
    if subslice_end > result.len() {
        eprintln!(
            "\rERROR: there were many more results than what the GPU could transfer to the CPU,\n\
             please use another tool for now, like https://www.openwall.com/php_mt_seed/"
        );
        return None;
    }
    let useful_results = &result[subslice_start..subslice_end];

    return Some(Vec::from(useful_results));
}

fn main() {
    let mut arguments = get_arguments();
    normalize_arguments(&mut arguments);
    if !lint_arguments(&arguments) {
        print_usage();
        return;
    }

    // wgpu uses `log` for all of our logging, so we initialize a logger with the `env_logger` crate.
    //
    // To change the log level, set the `RUST_LOG` environment variable. See the `env_logger`
    // documentation for more information.
    env_logger::init();

    for step in 0..256 {
        match find_mersenne_seed(&arguments, step) {
            None => std::process::exit(1),
            Some(results) => {
                for seed in results {
                    println!("\rseed = {:#x} = {} (PHP 7.1.0+)", seed, seed);
                }

                print!("\rprogress: {:03} / 256", step + 1);
                io::stdout().flush().unwrap();
            }
        }
    }

    println!("");
}

#[test]
fn test_find_seed_0() {
    let mut arguments = vec![1178568022];
    let expected_seed = 0;
    normalize_arguments(&mut arguments);
    let step = expected_seed % 256;
    let result = find_mersenne_seed(&arguments, step);
    assert_eq!(result, Some(vec![expected_seed]));
}

#[test]
fn test_find_seed_0_short_range() {
    let mut arguments = vec![16378811, 16378811, 0, 21474836];
    let expected_seed = 0;
    normalize_arguments(&mut arguments);
    let step = expected_seed % 256;
    let result = find_mersenne_seed(&arguments, step).unwrap();
    assert!(
        result.contains(&expected_seed),
        "expected that the results contain the seed {expected_seed} : {result:?}"
    );
}

#[test]
fn test_lint_too_big_range() {
    let arguments = vec![
        1395647406, 1395647406, 0, 4294967295, 3472777710, 3472777710, 0, 4294967295, 4039049869,
        4039049869, 0, 4294967295,
    ];
    assert_eq!(false, lint_arguments(&arguments));
}

#[test]
fn test_find_seed_with_multiple_outputs_default_range() {
    let arguments = vec![
        697823703, 697823703, 0, 0x7fffffff, 1736388855, 1736388855, 0, 0x7fffffff, 2019524934,
        2019524934, 0, 0x7fffffff,
    ];
    let expected_seed = 4242;
    let step = expected_seed % 256;
    let result = find_mersenne_seed(&arguments, step);
    assert_eq!(result, Some(vec![expected_seed]));
}

#[test]
fn test_find_seed_with_multiple_outputs_shorter_ranges() {
    let arguments = vec![
        7505, 7505, 1000, 10000, 2986, 2986, 1000, 10000, 1457, 1457, 1000, 10000,
    ];
    let expected_seed = 424242;
    let step = expected_seed % 256;
    let result = find_mersenne_seed(&arguments, step);
    assert_eq!(result, Some(vec![expected_seed]));
}
