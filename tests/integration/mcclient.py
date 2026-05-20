"""Tiny Minecraft Java client used by prune-inhabited integration tests."""

from __future__ import annotations

import json
import logging
import socket
import struct
import threading
import time
import uuid
import zlib
from dataclasses import dataclass
from hashlib import md5

log = logging.getLogger(__name__)

CONNECT_TIMEOUT_SECONDS = 30
JOIN_TIMEOUT_SECONDS = 90


class Buffer:
    def __init__(self, data: bytes = b"") -> None:
        self.data = data
        self.pos = 0

    def remaining(self) -> bytes:
        return self.data[self.pos :]

    def read(self, n: int) -> bytes:
        if self.pos + n > len(self.data):
            raise EOFError("packet ended early")
        out = self.data[self.pos : self.pos + n]
        self.pos += n
        return out

    def u8(self) -> int:
        return self.read(1)[0]

    def bool(self) -> bool:
        return self.u8() != 0

    def i32(self) -> int:
        return struct.unpack(">i", self.read(4))[0]

    def i64(self) -> int:
        return struct.unpack(">q", self.read(8))[0]

    def f32(self) -> float:
        return struct.unpack(">f", self.read(4))[0]

    def f64(self) -> float:
        return struct.unpack(">d", self.read(8))[0]

    def varint(self) -> int:
        return decode_varint_from(self)

    def string(self) -> str:
        n = self.varint()
        return self.read(n).decode("utf-8")


def encode_varint(value: int) -> bytes:
    value &= 0xFFFFFFFF
    out = bytearray()
    while True:
        b = value & 0x7F
        value >>= 7
        if value:
            out.append(b | 0x80)
        else:
            out.append(b)
            return bytes(out)


def decode_varint_from(buf: Buffer) -> int:
    num_read = 0
    result = 0
    while True:
        byte = buf.u8()
        result |= (byte & 0x7F) << (7 * num_read)
        num_read += 1
        if num_read > 5:
            raise ValueError("VarInt is too big")
        if byte & 0x80 == 0:
            if result & (1 << 31):
                result -= 1 << 32
            return result


def encode_string(value: str) -> bytes:
    raw = value.encode("utf-8")
    return encode_varint(len(raw)) + raw


def recv_exact(sock: socket.socket, n: int) -> bytes:
    chunks = bytearray()
    while len(chunks) < n:
        chunk = sock.recv(n - len(chunks))
        if not chunk:
            raise EOFError("socket closed")
        chunks.extend(chunk)
    return bytes(chunks)


def recv_varint(sock: socket.socket) -> int:
    return decode_varint_from(SocketBuffer(sock))


class SocketBuffer:
    def __init__(self, sock: socket.socket) -> None:
        self.sock = sock

    def u8(self) -> int:
        return recv_exact(self.sock, 1)[0]


def offline_uuid(username: str) -> uuid.UUID:
    digest = bytearray(md5(f"OfflinePlayer:{username}".encode("utf-8")).digest())
    digest[6] = (digest[6] & 0x0F) | 0x30
    digest[8] = (digest[8] & 0x3F) | 0x80
    return uuid.UUID(bytes=bytes(digest))


@dataclass(frozen=True)
class ProtocolSpec:
    family: str
    position_format: str = "legacy"
    movement_format: str = "bool"
    login_start_uuid: bool = False
    login_start_uuid_optional: bool = False
    login_success: int = 0x02
    login_set_compression: int | None = 0x03
    login_ack: int | None = None
    config_finish_clientbound: int | None = None
    config_finish_serverbound: int | None = None
    config_keepalive_clientbound: int | None = None
    config_keepalive_serverbound: int | None = None
    config_ping_clientbound: int | None = None
    config_pong_serverbound: int | None = None
    config_select_known_packs_clientbound: int | None = None
    config_select_known_packs_serverbound: int | None = None
    play_login: int | None = None
    play_keepalive_clientbound: int | None = None
    play_keepalive_serverbound: int | None = None
    play_position_clientbound: int | None = None
    play_teleport_confirm_serverbound: int | None = None
    play_move_pos_rot_serverbound: int | None = None
    play_move_status_serverbound: int | None = None
    play_chunk_batch_finished_clientbound: int | None = None
    play_chunk_batch_received_serverbound: int | None = None
    play_player_loaded_serverbound: int | None = None


LEGACY_1_7 = ProtocolSpec(
    family="1.7",
    position_format="1.7",
    movement_format="legacy_stance",
    login_set_compression=None,
    play_login=0x01,
    play_keepalive_clientbound=0x00,
    play_keepalive_serverbound=0x00,
    play_position_clientbound=0x08,
    play_move_pos_rot_serverbound=0x06,
    play_move_status_serverbound=0x03,
)

LEGACY_1_12 = ProtocolSpec(
    family="1.12",
    position_format="legacy",
    play_login=0x23,
    play_keepalive_clientbound=0x1F,
    play_keepalive_serverbound=0x0B,
    play_position_clientbound=0x2F,
    play_teleport_confirm_serverbound=0x00,
    play_move_pos_rot_serverbound=0x0E,
    play_move_status_serverbound=0x0C,
)

MODERN_1_14_4 = ProtocolSpec(
    family="modern",
    position_format="legacy",
    play_login=0x25,
    play_keepalive_clientbound=0x20,
    play_keepalive_serverbound=0x0F,
    play_position_clientbound=0x35,
    play_teleport_confirm_serverbound=0x00,
    play_move_pos_rot_serverbound=0x12,
    play_move_status_serverbound=0x14,
)

MODERN_1_15_2 = ProtocolSpec(
    family="modern",
    position_format="legacy",
    play_login=0x26,
    play_keepalive_clientbound=0x21,
    play_keepalive_serverbound=0x0F,
    play_position_clientbound=0x36,
    play_teleport_confirm_serverbound=0x00,
    play_move_pos_rot_serverbound=0x12,
    play_move_status_serverbound=0x14,
)

MODERN_1_13_2 = ProtocolSpec(
    family="modern",
    position_format="legacy",
    play_login=0x25,
    play_keepalive_clientbound=0x21,
    play_keepalive_serverbound=0x0E,
    play_position_clientbound=0x32,
    play_teleport_confirm_serverbound=0x00,
    play_move_pos_rot_serverbound=0x11,
    play_move_status_serverbound=0x0F,
)

MODERN_1_16_5 = ProtocolSpec(
    family="modern",
    position_format="legacy",
    play_login=0x24,
    play_keepalive_clientbound=0x1F,
    play_keepalive_serverbound=0x10,
    play_position_clientbound=0x34,
    play_teleport_confirm_serverbound=0x00,
    play_move_pos_rot_serverbound=0x13,
    play_move_status_serverbound=0x15,
)

MODERN_1_17_1 = ProtocolSpec(
    family="modern",
    position_format="legacy",
    play_login=0x26,
    play_keepalive_clientbound=0x21,
    play_keepalive_serverbound=0x0F,
    play_position_clientbound=0x38,
    play_teleport_confirm_serverbound=0x00,
    play_move_pos_rot_serverbound=0x12,
    play_move_status_serverbound=0x14,
)

MODERN_1_18_2 = ProtocolSpec(
    family="modern",
    position_format="legacy",
    play_login=0x26,
    play_keepalive_clientbound=0x21,
    play_keepalive_serverbound=0x0F,
    play_position_clientbound=0x38,
    play_teleport_confirm_serverbound=0x00,
    play_move_pos_rot_serverbound=0x12,
    play_move_status_serverbound=0x14,
)

MODERN_1_19_4 = ProtocolSpec(
    family="modern",
    position_format="legacy",
    login_start_uuid_optional=True,
    play_login=0x28,
    play_keepalive_clientbound=0x23,
    play_keepalive_serverbound=0x12,
    play_position_clientbound=0x3C,
    play_teleport_confirm_serverbound=0x00,
    play_move_pos_rot_serverbound=0x15,
    play_move_status_serverbound=0x17,
)

MODERN_1_20_6 = ProtocolSpec(
    family="modern",
    position_format="legacy",
    login_start_uuid=True,
    login_ack=0x03,
    config_finish_clientbound=0x03,
    config_finish_serverbound=0x03,
    config_keepalive_clientbound=0x04,
    config_keepalive_serverbound=0x04,
    config_ping_clientbound=0x05,
    config_pong_serverbound=0x05,
    config_select_known_packs_clientbound=0x0E,
    config_select_known_packs_serverbound=0x07,
    play_login=0x2B,
    play_keepalive_clientbound=0x26,
    play_keepalive_serverbound=0x18,
    play_position_clientbound=0x40,
    play_teleport_confirm_serverbound=0x00,
    play_move_pos_rot_serverbound=0x1B,
    play_move_status_serverbound=0x1D,
    play_chunk_batch_finished_clientbound=0x0C,
    play_chunk_batch_received_serverbound=0x08,
)

MODERN_1_21_8 = ProtocolSpec(
    family="modern",
    position_format="current",
    movement_format="flags",
    login_start_uuid=True,
    login_ack=0x03,
    config_finish_clientbound=0x03,
    config_finish_serverbound=0x03,
    config_keepalive_clientbound=0x04,
    config_keepalive_serverbound=0x04,
    config_ping_clientbound=0x05,
    config_pong_serverbound=0x05,
    config_select_known_packs_clientbound=0x0E,
    config_select_known_packs_serverbound=0x07,
    play_login=0x2B,
    play_keepalive_clientbound=0x26,
    play_keepalive_serverbound=0x1B,
    play_position_clientbound=0x41,
    play_teleport_confirm_serverbound=0x00,
    play_move_pos_rot_serverbound=0x1E,
    play_move_status_serverbound=0x20,
    play_chunk_batch_finished_clientbound=0x0B,
    play_chunk_batch_received_serverbound=0x0A,
    play_player_loaded_serverbound=0x2B,
)

MODERN_26_1_2 = ProtocolSpec(
    family="modern",
    position_format="current",
    movement_format="flags",
    login_start_uuid=True,
    login_ack=0x03,
    config_finish_clientbound=0x03,
    config_finish_serverbound=0x03,
    config_keepalive_clientbound=0x04,
    config_keepalive_serverbound=0x04,
    config_ping_clientbound=0x05,
    config_pong_serverbound=0x05,
    config_select_known_packs_clientbound=0x0E,
    config_select_known_packs_serverbound=0x07,
    play_login=0x31,
    play_keepalive_clientbound=0x2C,
    play_keepalive_serverbound=0x1C,
    play_position_clientbound=0x48,
    play_teleport_confirm_serverbound=0x00,
    play_move_pos_rot_serverbound=0x1F,
    play_move_status_serverbound=0x21,
    play_chunk_batch_finished_clientbound=0x0B,
    play_chunk_batch_received_serverbound=0x0B,
    play_player_loaded_serverbound=0x2C,
)


PROTOCOL_SPECS = {
    5: LEGACY_1_7,
    340: LEGACY_1_12,
    404: MODERN_1_13_2,
    498: MODERN_1_14_4,
    578: MODERN_1_15_2,
    754: MODERN_1_16_5,
    756: MODERN_1_17_1,
    758: MODERN_1_18_2,
    762: MODERN_1_19_4,
    766: MODERN_1_20_6,
    767: MODERN_1_20_6,
    772: MODERN_1_21_8,
    775: MODERN_26_1_2,
}


def protocol_spec(protocol: int, name: str) -> ProtocolSpec:
    if protocol in PROTOCOL_SPECS:
        return PROTOCOL_SPECS[protocol]
    if protocol >= 775 or name.startswith("26."):
        return MODERN_26_1_2
    if protocol >= 772 or name.startswith("1.21.8"):
        return MODERN_1_21_8
    raise RuntimeError(f"unsupported Minecraft protocol {protocol} ({name})")


def packet(packet_id: int, payload: bytes = b"") -> bytes:
    body = encode_varint(packet_id) + payload
    return encode_varint(len(body)) + body


def handshake(protocol: int, host: str, port: int, next_state: int) -> bytes:
    body = (
        encode_varint(protocol)
        + encode_string(host)
        + struct.pack(">H", port)
        + encode_varint(next_state)
    )
    return packet(0x00, body)


def query_status(host: str, port: int) -> dict:
    last_error: BaseException | None = None
    deadline = time.monotonic() + CONNECT_TIMEOUT_SECONDS
    while time.monotonic() < deadline:
        for protocol in [775, 772, 767, 766, 758, 754, 404, 340, 5, 0]:
            try:
                return _query_status_with_protocol(host, port, protocol)
            except (EOFError, OSError, RuntimeError) as e:
                last_error = e
        time.sleep(0.25)
    raise RuntimeError("server did not answer status ping") from last_error


def _query_status_with_protocol(host: str, port: int, protocol: int) -> dict:
    with socket.create_connection((host, port), timeout=CONNECT_TIMEOUT_SECONDS) as sock:
        sock.sendall(handshake(protocol, host, port, 1))
        sock.sendall(packet(0x00))
        length = recv_varint(sock)
        payload = Buffer(recv_exact(sock, length))
        packet_id = payload.varint()
        if packet_id != 0x00:
            raise RuntimeError(f"unexpected status packet id {packet_id}")
        return json.loads(payload.string())


class MinecraftClient:
    def __init__(self, host: str, port: int, username: str = "McmapBot") -> None:
        self.host = host
        self.port = port
        self.username = username
        self.status = query_status(host, port)
        version = self.status["version"]
        self.protocol = int(version["protocol"])
        self.version_name = str(version["name"])
        self.spec = protocol_spec(self.protocol, self.version_name)
        self.sock: socket.socket | None = None
        self.state = "login"
        self.compression_threshold: int | None = None
        self.joined = threading.Event()
        self.closed = threading.Event()
        self.error: BaseException | None = None
        self._thread: threading.Thread | None = None
        self._move_thread: threading.Thread | None = None
        self._send_lock = threading.Lock()
        self._position_lock = threading.Lock()
        self._position: tuple[float, float, float] | None = None
        self._yaw = 0.0
        self._pitch = 0.0

    def __enter__(self) -> "MinecraftClient":
        self.start()
        return self

    def __exit__(self, exc_type, exc, tb) -> None:
        self.close()

    def start(self) -> None:
        self.sock = socket.create_connection(
            (self.host, self.port), timeout=CONNECT_TIMEOUT_SECONDS
        )
        self.sock.settimeout(10)
        self.sock.sendall(handshake(self.protocol, self.host, self.port, 2))
        self._send_login_start()
        self._thread = threading.Thread(target=self._read_loop, daemon=True)
        self._thread.start()
        if not self.joined.wait(JOIN_TIMEOUT_SECONDS):
            self.close()
            if self.error is not None:
                raise RuntimeError("Minecraft client failed to join") from self.error
            raise TimeoutError(
                f"Minecraft client did not reach play state on {self.version_name}"
            )
        self._move_thread = threading.Thread(target=self._movement_loop, daemon=True)
        self._move_thread.start()

    def close(self) -> None:
        self.closed.set()
        sock = self.sock
        if sock is not None:
            try:
                sock.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            try:
                sock.close()
            except OSError:
                pass
            self.sock = None
        if self._thread is not None:
            self._thread.join(timeout=5)
            self._thread = None
        if self._move_thread is not None:
            self._move_thread.join(timeout=5)
            self._move_thread = None

    def _send_login_start(self) -> None:
        payload = encode_string(self.username)
        if self.spec.login_start_uuid:
            payload += offline_uuid(self.username).bytes
        if self.spec.login_start_uuid_optional:
            payload += b"\x00"
        self.send_packet(0x00, payload)

    def send_packet(self, packet_id: int, payload: bytes = b"") -> None:
        sock = self.sock
        if sock is None:
            raise RuntimeError("client is not connected")
        body = encode_varint(packet_id) + payload
        if self.compression_threshold is not None:
            body = b"\x00" + body
        with self._send_lock:
            sock.sendall(encode_varint(len(body)) + body)

    def _read_loop(self) -> None:
        try:
            while not self.closed.is_set():
                packet_id, payload = self._recv_packet()
                self._handle_packet(packet_id, payload)
        except (EOFError, OSError) as e:
            if not self.closed.is_set():
                self.error = e
        except BaseException as e:
            self.error = e
            log.exception("Minecraft client reader failed")
        finally:
            self.closed.set()

    def _recv_packet(self) -> tuple[int, Buffer]:
        sock = self.sock
        if sock is None:
            raise EOFError("client is closed")
        length = recv_varint(sock)
        frame = recv_exact(sock, length)
        if self.compression_threshold is not None:
            buf = Buffer(frame)
            data_length = buf.varint()
            rest = buf.remaining()
            frame = rest if data_length == 0 else zlib.decompress(rest)
        buf = Buffer(frame)
        packet_id = buf.varint()
        return packet_id, buf

    def _handle_packet(self, packet_id: int, payload: Buffer) -> None:
        if self.state == "login":
            self._handle_login(packet_id, payload)
        elif self.state == "configuration":
            self._handle_configuration(packet_id, payload)
        else:
            self._handle_play(packet_id, payload)

    def _handle_login(self, packet_id: int, payload: Buffer) -> None:
        if (
            self.spec.login_set_compression is not None
            and packet_id == self.spec.login_set_compression
        ):
            self.compression_threshold = payload.varint()
            return
        if packet_id == 0x00:
            raise RuntimeError(f"login disconnect: {payload.remaining()!r}")
        if packet_id != self.spec.login_success:
            return
        if self.spec.login_ack is not None:
            self.send_packet(self.spec.login_ack)
            self.state = "configuration"
        else:
            self.state = "play"

    def _handle_configuration(self, packet_id: int, payload: Buffer) -> None:
        spec = self.spec
        if packet_id == spec.config_finish_clientbound:
            self.send_packet(spec.config_finish_serverbound or 0)
            self.state = "play"
            return
        if packet_id == spec.config_keepalive_clientbound:
            self.send_packet(spec.config_keepalive_serverbound or 0, payload.remaining())
            return
        if packet_id == spec.config_ping_clientbound:
            self.send_packet(spec.config_pong_serverbound or 0, payload.remaining())
            return
        if packet_id == spec.config_select_known_packs_clientbound:
            self.send_packet(
                spec.config_select_known_packs_serverbound or 0,
                payload.remaining(),
            )
            return

    def _handle_play(self, packet_id: int, payload: Buffer) -> None:
        spec = self.spec
        if packet_id == spec.play_login:
            self.joined.set()
            self._send_player_loaded()
            return
        if packet_id == spec.play_keepalive_clientbound:
            self.send_packet(spec.play_keepalive_serverbound or 0, payload.remaining())
            return
        if packet_id == spec.play_position_clientbound:
            self._handle_position(payload)
            return
        if packet_id == spec.play_chunk_batch_finished_clientbound:
            self.send_packet(
                spec.play_chunk_batch_received_serverbound or 0,
                struct.pack(">f", 20.0),
            )

    def _handle_position(self, payload: Buffer) -> None:
        spec = self.spec
        if spec.position_format == "1.7":
            x = payload.f64()
            y = payload.f64()
            z = payload.f64()
            yaw = payload.f32()
            pitch = payload.f32()
            payload.bool()
            self._set_position(x, y, z, yaw, pitch)
            return
        if spec.position_format == "legacy":
            x = payload.f64()
            y = payload.f64()
            z = payload.f64()
            yaw = payload.f32()
            pitch = payload.f32()
            payload.u8()
            teleport_id = payload.varint()
            self._set_position(x, y, z, yaw, pitch)
            self.send_packet(
                spec.play_teleport_confirm_serverbound or 0,
                encode_varint(teleport_id),
            )
            return
        if spec.position_format == "current":
            teleport_id = payload.varint()
            x = payload.f64()
            y = payload.f64()
            z = payload.f64()
            payload.f64()
            payload.f64()
            payload.f64()
            yaw = payload.f32()
            pitch = payload.f32()
            self._set_position(x, y, z, yaw, pitch)
        else:
            x = payload.f64()
            y = payload.f64()
            z = payload.f64()
            yaw = payload.f32()
            pitch = payload.f32()
            payload.u8()
            teleport_id = payload.varint()
            self._set_position(x, y, z, yaw, pitch)
        self.send_packet(
            spec.play_teleport_confirm_serverbound or 0,
            encode_varint(teleport_id),
        )

    def _send_player_loaded(self) -> None:
        if self.spec.play_player_loaded_serverbound is None:
            return
        self.send_packet(self.spec.play_player_loaded_serverbound)

    def _set_position(
        self, x: float, y: float, z: float, yaw: float, pitch: float
    ) -> None:
        with self._position_lock:
            self._position = (x, y, z)
            self._yaw = yaw
            self._pitch = pitch

    def _movement_loop(self) -> None:
        while not self.closed.wait(0.05):
            if self.state != "play":
                continue
            try:
                self._send_movement()
            except OSError as e:
                if not self.closed.is_set():
                    self.error = e
                    self.closed.set()
                return

    def _send_movement(self) -> None:
        spec = self.spec
        flags = b"\x01"
        if spec.play_move_status_serverbound is not None:
            self.send_packet(spec.play_move_status_serverbound, flags)
            return
        with self._position_lock:
            position = self._position
            yaw = self._yaw
            pitch = self._pitch
        if position is not None and spec.play_move_pos_rot_serverbound is not None:
            x, y, z = position
            if spec.movement_format == "legacy_stance":
                payload = struct.pack(">ddddff?", x, y + 1.62, y, z, yaw, pitch, True)
            elif spec.movement_format == "flags":
                payload = struct.pack(">dddff", x, y, z, yaw, pitch) + flags
            else:
                payload = struct.pack(">dddff?", x, y, z, yaw, pitch, True)
            self.send_packet(spec.play_move_pos_rot_serverbound, payload)
            return
