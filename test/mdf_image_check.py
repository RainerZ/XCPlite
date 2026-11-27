#!/usr/bin/env python3
"""Check the image of multi_thread_demo in an MDF file written by xcpclient (test_multi_thread_image.sh).

Usage: mdf_image_check.py <file.mf4> <work.c>

Reads the array channel 'pixels' (a CN block with a CA block, written by the MDF writer of xcpclient), takes the last
sample and compares every 10th line with a reference render of the Mandelbrot set. The area (mandelbrot.x, .y, .width),
the resolution and the iteration limit are the defaults in examples/multi_thread_demo/src/work.c.

Floating point rounding (FMA contraction of the compiler) may change the iteration count of single pixels at the
boundary of the set, so isolated differing pixels are accepted. A transport or reassembly error would corrupt whole
ODT entries (up to 248 bytes) or ODTs. Exit status 0 if the image matches, 1 if not, 2 on a usage or file error.
Only the Python standard library is used.
"""

import re
import struct
import sys

LINE_STEP = 10  # Compare every 10th line


def fail(message, status=1):
    print(f"FAIL: {message}")
    sys.exit(status)


def read_mdf_channel(path, channel_name):
    """Returns (dims, samples) of an array channel: dims from the CA block (row oriented), samples the payloads"""
    data = open(path, "rb").read()
    if data[0:8] not in (b"MDF     ", b"UnFinMF "):
        fail(f"{path} is not an MDF file", 2)

    def block(pos):
        if data[pos:pos + 2] != b"##":
            fail(f"no MDF block at 0x{pos:X}", 2)
        length, link_count = struct.unpack_from("<QQ", data, pos + 8)
        links = list(struct.unpack_from(f"<{link_count}Q", data, pos + 24))
        return data[pos + 2:pos + 4], links, data[pos + 24 + 8 * link_count:pos + length]

    def text(pos):
        return block(pos)[2].split(b"\0")[0].decode()

    _, hd_links, _ = block(64)
    _, dg_links, dg_data = block(hd_links[0])
    record_id_size = dg_data[0]
    if record_id_size != 2:
        fail(f"unexpected record id size {record_id_size}", 2)
    dt_id, _, dt = block(dg_links[2])
    if dt_id != b"DT":
        fail("no DT block", 2)

    # Channel groups: record id -> record size, find the channel
    record_sizes = {}
    found = None
    cg = dg_links[1]
    while cg:
        _, cg_links, cg_data = block(cg)
        record_id = struct.unpack_from("<Q", cg_data, 0)[0]
        record_sizes[record_id] = struct.unpack_from("<I", cg_data, 24)[0]
        cn = cg_links[1]
        while cn:
            _, cn_links, cn_data = block(cn)
            if cn_links[2] and text(cn_links[2]) == channel_name:
                byte_offset, bit_count = struct.unpack_from("<II", cn_data, 4)
                if not cn_links[1]:
                    fail(f"channel {channel_name} has no CA block", 2)
                ca_id, _, ca_data = block(cn_links[1])
                ndim = struct.unpack_from("<H", ca_data, 2)[0]
                dims = struct.unpack_from(f"<{ndim}Q", ca_data, 16)
                found = (record_id, byte_offset, bit_count // 8, dims)
            cn = cn_links[0]
        cg = cg_links[0]
    if found is None:
        fail(f"channel {channel_name} not found", 2)
    record_id, byte_offset, element_size, dims = found
    if element_size != 1:
        fail(f"unexpected element size {element_size}", 2)
    size = 1
    for d in dims:
        size *= d

    # Records: record id (u16), record data, the channel at byte_offset (behind the record id)
    samples = []
    pos = 0
    while pos + 2 <= len(dt):
        rid = struct.unpack_from("<H", dt, pos)[0]
        if rid not in record_sizes:
            fail(f"unknown record id {rid} at DT offset {pos}", 2)
        if rid == record_id:
            samples.append(dt[pos + 2 + byte_offset:pos + 2 + byte_offset + size])
        pos += 2 + record_sizes[rid]
    return dims, samples


def read_work_defaults(path):
    """Resolution, iteration limit and default area from work.c"""
    src = open(path).read()

    def define(name):
        m = re.search(rf"^#define {name} (\d+)", src, re.MULTILINE)
        if not m:
            fail(f"#define {name} not found in {path}", 2)
        return int(m.group(1))

    m = re.search(r"^static const mandelbrot_t mandelbrot = \{\.x = ([-+.\deE]+), \.y = ([-+.\deE]+), \.width = ([-+.\deE]+)", src, re.MULTILINE)
    if not m:
        fail(f"default parameters of mandelbrot not found in {path}", 2)
    return define("X_RES"), define("Y_RES"), define("MAX_ITER"), float(m.group(1)), float(m.group(2)), float(m.group(3))


def iterations(cr, ci, max_iter):
    """Same iteration as mandelbrot_iterations in work.c"""
    zr = zi = 0.0
    for i in range(max_iter):
        zr2, zi2 = zr * zr, zi * zi
        if zr2 + zi2 > 4.0:
            return i
        zi = 2.0 * zr * zi + ci
        zr = zr2 - zi2 + cr
    return max_iter + 1


def main():
    if len(sys.argv) != 3:
        print(__doc__)
        sys.exit(2)
    dims, samples = read_mdf_channel(sys.argv[1], "pixels")
    x_res, y_res, max_iter, x, y, width = read_work_defaults(sys.argv[2])
    if tuple(dims) != (y_res, x_res):
        fail(f"MDF dimensions {list(dims)} do not match the resolution {x_res}x{y_res} of work.c (expected [{y_res}, {x_res}])")
    if not samples:
        fail("no sample of pixels in the MDF file")
    image = samples[-1]
    if len(image) != x_res * y_res:
        fail(f"last sample has {len(image)} bytes, expected {x_res * y_res}")

    # Reference render of every LINE_STEP-th line, the pixel size and the upper left corner as in work_begin_batch
    pixel_size = width / x_res
    upper_left_re = x - width / 2.0
    upper_left_im = y + pixel_size * y_res / 2.0
    lines = list(range(0, y_res, LINE_STEP))
    if lines[-1] != y_res - 1:
        lines.append(y_res - 1)
    differences = []
    for line in lines:
        ci = upper_left_im - line * pixel_size
        row = image[line * x_res:(line + 1) * x_res]
        for column in range(x_res):
            expected = iterations(upper_left_re + column * pixel_size, ci, max_iter)
            if row[column] != expected:
                differences.append((line, column, row[column], expected))

    compared = len(lines) * x_res
    print(f"{len(samples)} samples of pixels [{x_res}x{y_res}], last sample: {compared} pixels in {len(lines)} lines compared, {len(differences)} differ")
    for line, column, value, expected in differences[:10]:
        print(f"  line {line}, column {column}: {value}, reference {expected}")
    adjacent = any(a[0] == b[0] and b[1] == a[1] + 1 for a, b in zip(differences, differences[1:]))
    if len(differences) > max(4, compared // 10000) or adjacent:
        fail("the image does not match the reference render (more than isolated rounding differences)")
    print("PASS")


if __name__ == "__main__":
    main()
