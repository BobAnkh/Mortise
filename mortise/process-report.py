#!/usr/bin/env python3

from utils.calc_opt_delta import FlowCtrl
import time
import socket
import os
import struct
import json
import structlog
import multiprocessing
from multiprocessing import Process
import logging
import socketserver

# import sys
# import ctypes as ct
# import array

socket_path = "/tmp/mortise-py.sock"
server_address = "/tmp/mortise.sock"


def add_callsite_info(logger, method_name, event_dict):
    """
    Add the callsite info to the event dict.
    """
    event_dict["event"] = (
        f"[{event_dict['filename']}:{event_dict['func_name']}:{event_dict['lineno']}] {event_dict['event']}"
    )
    del event_dict["filename"]
    del event_dict["func_name"]
    del event_dict["lineno"]
    return event_dict


level = os.environ.get("LOG_LEVEL", "INFO").upper()
LOG_LEVEL = getattr(logging, level)
structlog.configure(
    processors=[
        structlog.processors.TimeStamper(fmt="%Y-%m-%d %H:%M.%S"),
        structlog.processors.add_log_level,
        # structlog.processors.CallsiteParameterAdder(
        #         [structlog.processors.CallsiteParameter.FILENAME,
        #         structlog.processors.CallsiteParameter.FUNC_NAME,
        #         structlog.processors.CallsiteParameter.LINENO],
        #     ),
        # add_callsite_info,
        structlog.stdlib.PositionalArgumentsFormatter(),
        structlog.processors.StackInfoRenderer(),
        structlog.processors.format_exc_info,
        structlog.dev.ConsoleRenderer(),
    ],
    wrapper_class=structlog.make_filtering_bound_logger(LOG_LEVEL),
)
logger = structlog.get_logger()


def send_and_receive_message(sock, message_dict):
    # Convert the message to bytes
    message = json.dumps(message_dict)
    message_bytes = message.encode("utf-8")

    # Get the length of the message in bytes
    message_length = len(message_bytes)

    # Pack the length as a 32-bit unsigned integer
    message_length_packed = struct.pack("!I", message_length)

    # Send the length followed by the message
    sock.sendall(message_length_packed + message_bytes)

    # Receive a message in the same format
    data_length_packed = sock.recv(4)  # Receive 4 bytes for the length
    data_length = struct.unpack("!I", data_length_packed)[0]  # Unpack the length

    # Receive the data
    data_bytes = sock.recv(data_length)  # Receive data of `data_length` length

    # Deserialize the data
    data_dict = json.loads(data_bytes.decode("utf-8"))
    # logger.info(f"Received: {data_dict}")


def worker(tx, rx, flow_id, sock):
    logger = structlog.get_logger()
    tx.close()
    flow_controller = FlowCtrl("FILE")
    while True:
        try:
            message = rx.recv()
            # Add message process logic here
            # print(f"Process {flow_id} received message: {message}")
            # message_dict = {"Manager": "PingPong"}
            flow_controller.add_data(message)
            message_dict = flow_controller.process()
            if message_dict != None:
                send_and_receive_message(sock, message_dict)
        except EOFError:
            logger.info(f"Exit process for {flow_id}.")
            sock.close()
            break


# UINT_10 = ct.c_uint * 10

# class ReportEntry(ct.Structure):
#     _fields_ = (
#         ("id", ct.c_uint),
#         ("chunk_id", ct.c_uint),
#         ("data_array", UINT_10),
#     )


class ReportDataElem:
    def __init__(self, rtt, acked_bytes, lost_bytes, timestamp):
        self.rtt = rtt
        self.acked_bytes = acked_bytes
        self.lost_bytes = lost_bytes
        self.timestamp = timestamp


class ReportEntry:
    flow_id = 0
    chunk_id = 0
    chunk_len = 0
    data_array = []


# total length: 808 = 4(u32) + 4(u16+s16) + 50 * 16(u32 + u32 + u64)
# header format
fmt = "<IhH"
fmt_size = struct.calcsize(fmt)
# print(fmt_size)

try:
    os.unlink(socket_path)
except OSError:
    pass

# s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
# s.bind(socket_path)
# os.chmod(socket_path, 0o666)
# s.listen(128)

flow_manager = {}
flow_locks = {}

# mortise_server_connected = False


class ThreadedUnixStreamHandler(socketserver.BaseRequestHandler):
    def handle(self):
        conn = self.request
        # try:
        while 1:
            data_length_packed = conn.recv(4)  # Receive 4 bytes for the length
            if not data_length_packed:
                break
            msg_len = struct.unpack("!I", data_length_packed)[0]  # Unpack the length
            data = conn.recv(msg_len)
            if data:
                if msg_len < 64:
                    # Deserialize the data
                    ctrl_data = json.loads(data.decode("utf-8"))
                    logger.debug(ctrl_data)
                    if "Disconnect" in ctrl_data:
                        flow_disconnect = ctrl_data["Disconnect"]
                        flow_id = flow_disconnect["flow_id"]
                        if flow_id in flow_locks:
                            lock = flow_locks[flow_id]
                            with lock:
                                if flow_id in flow_manager:
                                    chan = flow_manager[flow_id]
                                    chan.close()
                                    del flow_manager[flow_id]
                                del flow_locks[flow_id]
                    elif "Connect" in ctrl_data:
                        flow_id = ctrl_data["Connect"]["flow_id"]
                        # print(flow_id, type(flow_id))
                        if flow_id in flow_locks:
                            lock = flow_locks[flow_id]
                            with lock:
                                if flow_id in flow_manager:
                                    chan = flow_manager[flow_id]
                                    chan.close()
                                    del flow_manager[flow_id]
                                del flow_locks[flow_id]
                            logger.info(f"Disconnected existing flow {flow_id}")
                        mtx = multiprocessing.Lock()
                        with mtx:
                            flow_locks[flow_id] = mtx
                            tx, rx = multiprocessing.Pipe()
                            sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                            sock.connect(server_address)
                            p = Process(target=worker, args=(tx, rx, flow_id, sock))
                            p.start()
                            flow_manager[flow_id] = tx
                        # print(f"Flow {flow_id} disconnected.")
                else:
                    # if not mortise_server_connected:
                    #     sock.connect(server_address)
                    #     mortise_server_connected = True
                    #     print(f'connecting to {server_address}')
                    # print("received ", data)
                    # x = ReportEntry()
                    report_entry = ReportEntry()
                    # U = array.array("10I")
                    # print(type(x.data_array))
                    (
                        report_entry.flow_id,
                        report_entry.chunk_id,
                        report_entry.chunk_len,
                    ) = struct.unpack(fmt, data[:fmt_size])
                    # U = struct.unpack("<10I", data[fmt_size:48])
                    report_entry.data_array = [
                        ReportDataElem(x[0], x[1], x[2], x[3])
                        for x in struct.iter_unpack("<IIII", data[fmt_size:msg_len])
                    ]
                    # if report_entry.flow_id not in flow_manager:
                    # tx, rx = multiprocessing.Pipe()
                    # sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                    # sock.connect(server_address)
                    # p = Process(
                    #     target=worker, args=(tx, rx, report_entry.flow_id, sock)
                    # )
                    # p.start()
                    # flow_manager[report_entry.flow_id] = tx
                    if report_entry.flow_id in flow_locks:
                        with flow_locks[report_entry.flow_id]:
                            if report_entry.flow_id in flow_manager:
                                # logger.info(f"recieve {report_entry.flow_id}")
                                chan = flow_manager[report_entry.flow_id]
                                chan.send(report_entry)

                    # print(type(y.data_array), type(y.id), type(y.data_array[0]), type(y.data_array[0].rtt))
                    # message_dict = {"Manager": "PingPong"}
                    # send_and_receive_message(sock, message_dict)
        # finally:
        #     # sock.close()
        #     conn.close()


logger.info(f"Listening to {socket_path}")
with socketserver.ThreadingUnixStreamServer(
    socket_path, ThreadedUnixStreamHandler
) as server:
    server.serve_forever()
