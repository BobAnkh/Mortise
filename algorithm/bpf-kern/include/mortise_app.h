#ifndef __MORTISE_APP_H
#define __MORTISE_APP_H
#include "vmlinux.h"
#include <linux/const.h>
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_core_read.h>
#include <bpf/bpf_tracing.h>

struct app_info {
	u64 req;
	u64 resp;
};

struct app_sk_stg {
	__uint(type, BPF_MAP_TYPE_SK_STORAGE);
	__uint(map_flags, BPF_F_NO_PREALLOC);
	__type(key, int);
	__type(value, struct app_info);
};

#endif