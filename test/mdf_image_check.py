#!/usr/bin/env python3
"""Check the images of multi_thread_demo in an MDF file written by xcpclient (test_multi_thread_image.sh).

Usage:
  mdf_image_check.py pixels <file.mf4> <work.c>   the uncompressed image 'pixels' (event batch_done)
  mdf_image_check.py jpeg <file.mf4> <work.c>     the JPEG image 'jpeg', 'jpeg_size', 'jpeg_image' (event jpeg)

pixels: takes the last sample and compares every 10th line with a reference render of the Mandelbrot set. The area
(mandelbrot.x, .y, .width), the resolution and the iteration limit are the defaults in examples/multi_thread_demo/src/work.c.
Floating point rounding (FMA contraction of the compiler) may change the iteration count of single pixels at the boundary of
the set, so isolated differing pixels are accepted. A transport or reassembly error would corrupt whole ODT entries (up to 248
bytes) or ODTs.

jpeg: checks each sample: jpeg_size of the sample with the same timestamp, the JPEG image starts with SOI, ends with EOI at
jpeg_size and has the resolution of the video (half of X_RES x Y_RES, SOF header), the image numbers increase. The JPEG image
is not decoded (only the Python standard library is used).

Exit status 0 if the images are correct, 1 if not, 2 on a usage or file error.
"""

import re
import struct
import sys

LINE_STEP = 10  # pixels: compare every 10th line


def fail(message, status=1):
    print(f"FAIL: {message}")
    sys.exit(status)


class Mdf:
    """The MDF file layout of the MDF writer of xcpclient: one data group, a channel group per DAQ list, records with a
    2 byte record id, an 8 byte timestamp and the payload of the DAQ list. Array channels have a CA block."""

    def __init__(self, path):
        self.data = open(path, "rb").read()
        if self.data[0:8] not in (b"MDF     ", b"UnFinMF "):
            fail(f"{path} is not an MDF file", 2)
        _, hd_links, _ = self.block(64)
        _, dg_links, dg_data = self.block(hd_links[0])
        if dg_data[0] != 2:
            fail(f"unexpected record id size {dg_data[0]}", 2)
        dt_id, _, dt = self.block(dg_links[2])
        if dt_id != b"DT":
            fail("no DT block", 2)

        # Channel groups: record id -> record size, channel name -> (record id, byte offset, element size, dims)
        record_sizes = {}
        self.channels = {}
        cg = dg_links[1]
        while cg:
            _, cg_links, cg_data = self.block(cg)
            record_id = struct.unpack_from("<Q", cg_data, 0)[0]
            record_sizes[record_id] = struct.unpack_from("<I", cg_data, 24)[0]
            cn = cg_links[1]
            while cn:
                _, cn_links, cn_data = self.block(cn)
                byte_offset, bit_count = struct.unpack_from("<II", cn_data, 4)
                dims = ()
                if cn_links[1]:
                    _, _, ca_data = self.block(cn_links[1])
                    ndim = struct.unpack_from("<H", ca_data, 2)[0]
                    dims = struct.unpack_from(f"<{ndim}Q", ca_data, 16)
                if cn_links[2]:
                    self.channels[self.text(cn_links[2])] = (record_id, byte_offset, bit_count // 8, dims)
                cn = cn_links[0]
            cg = cg_links[0]

        # Records of each record id: (raw timestamp, record data behind the record id)
        self.records = {record_id: [] for record_id in record_sizes}
        pos = 0
        while pos + 2 <= len(dt):
            rid = struct.unpack_from("<H", dt, pos)[0]
            if rid not in record_sizes:
                fail(f"unknown record id {rid} at DT offset {pos}", 2)
            record = dt[pos + 2:pos + 2 + record_sizes[rid]]
            self.records[rid].append((struct.unpack_from("<q", record, 0)[0], record))
            pos += 2 + record_sizes[rid]

    def block(self, pos):
        if self.data[pos:pos + 2] != b"##":
            fail(f"no MDF block at 0x{pos:X}", 2)
        length, link_count = struct.unpack_from("<QQ", self.data, pos + 8)
        links = list(struct.unpack_from(f"<{link_count}Q", self.data, pos + 24))
        return self.data[pos + 2:pos + 4], links, self.data[pos + 24 + 8 * link_count:pos + length]

    def text(self, pos):
        return self.block(pos)[2].split(b"\0")[0].decode()

    def channel(self, name):
        if name not in self.channels:
            fail(f"channel {name} not found", 2)
        return self.channels[name]

    def samples(self, name):
        """Samples of a channel: list of (timestamp, bytes of all elements)"""
        record_id, offset, element_size, dims = self.channel(name)
        size = element_size
        for d in dims:
            size *= d
        return [(t, record[offset:offset + size]) for t, record in self.records[record_id]]

    def values(self, name):
        """Samples of an unsigned scalar channel: dict timestamp -> value"""
        return {t: int.from_bytes(data, "little") for t, data in self.samples(name)}


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


def check_pixels(mdf, work_c):
    x_res, y_res, max_iter, x, y, width = read_work_defaults(work_c)
    dims = mdf.channel("pixels")[3]
    if tuple(dims) != (y_res, x_res):
        fail(f"MDF dimensions {list(dims)} do not match the resolution {x_res}x{y_res} of work.c (expected [{y_res}, {x_res}])")
    samples = mdf.samples("pixels")
    if not samples:
        fail("no sample of pixels in the MDF file")
    image = samples[-1][1]

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


def jpeg_resolution(data):
    """Width and height from the SOF header of a JPEG image, None if there is none"""
    pos = 2
    while pos + 9 <= len(data) and data[pos] == 0xFF:
        marker = data[pos + 1]
        length = struct.unpack_from(">H", data, pos + 2)[0]
        if marker in (0xC0, 0xC1, 0xC2):
            height, width = struct.unpack_from(">HH", data, pos + 5)
            return width, height
        pos += 2 + length
    return None


def check_jpeg(mdf, work_c):
    x_res, y_res = read_work_defaults(work_c)[0:2]
    resolution = (x_res // 2, y_res // 2)
    sizes = mdf.values("jpeg_size")
    numbers = mdf.values("jpeg_image")
    samples = mdf.samples("jpeg")
    if not samples:
        fail("no sample of jpeg in the MDF file")
    capacity = len(samples[0][1])
    previous = None
    for t, buffer in samples:
        if t not in sizes or t not in numbers:
            fail(f"no jpeg_size or jpeg_image with the timestamp {t} of a jpeg sample")
        size, number = sizes[t], numbers[t]
        image = buffer[:size]
        if not 4 <= size <= capacity or image[:2] != b"\xff\xd8" or image[-2:] != b"\xff\xd9":
            fail(f"image {number}: no complete JPEG image, jpeg_size {size}, buffer {capacity} bytes")
        if jpeg_resolution(image) != resolution:
            fail(f"image {number}: resolution {jpeg_resolution(image)}, expected {resolution}")
        if previous is not None and number <= previous:
            fail(f"image numbers do not increase: {previous}, {number}")
        previous = number
    print(f"{len(samples)} samples of jpeg (buffer {capacity} bytes): complete JPEG images {resolution[0]}x{resolution[1]}, "
          f"images {numbers[samples[0][0]]}..{previous}, {min(sizes.values())}..{max(sizes.values())} bytes")


def main():
    if len(sys.argv) != 4 or sys.argv[1] not in ("pixels", "jpeg"):
        print(__doc__)
        sys.exit(2)
    mdf = Mdf(sys.argv[2])
    if sys.argv[1] == "pixels":
        check_pixels(mdf, sys.argv[3])
    else:
        check_jpeg(mdf, sys.argv[3])
    print("PASS")


if __name__ == "__main__":
    main()
