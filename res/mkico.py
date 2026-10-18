# Frame Zero - a tiny Windows screenshot and annotation tool
# Copyright (C) 2026 Claude Henchoz
# SPDX-License-Identifier: GPL-3.0-or-later
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program.  If not, see <https://www.gnu.org/licenses/>.

import struct, sys
out, pngs = sys.argv[1], sys.argv[2:]
data = [open(p, 'rb').read() for p in pngs]
hdr = struct.pack('<HHH', 0, 1, len(data))
off = 6 + 16 * len(data)
dirs = b''
for d in data:
    w, h = struct.unpack('>II', d[16:24])
    dirs += struct.pack('<BBBBHHII', w & 255, h & 255, 0, 0, 1, 32, len(d), off)
    off += len(d)
open(out, 'wb').write(hdr + dirs + b''.join(data))
