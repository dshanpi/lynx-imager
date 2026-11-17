#include <ctype.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>


#include "efex-common.h"
#include "efex-protocol.h"
#include "efex-usb.h"
#include "ending.h"
#include "efex-fes.h"
#include "usb_layer.h"

/* MBR commit may erase/attach NAND UBI before returning its AWUS status.
 * Only the final MBR completion read gets this budget; never resend a timed-out write. */
#define FES_MBR_COMMIT_TIMEOUT_MS 60000


int sunxi_send_usb_request(const struct sunxi_efex_ctx_t *ctx, const enum sunxi_efex_usb_request_t type,
                           const size_t length) {
	if (!ctx || !ctx->hdl) {
		return EFEX_ERR_NULL_PTR;
	}

	struct sunxi_usb_request_t req = {
			.magics = SUNXI_USB_REQ_MAGIC_INT,
			.tab = 0x0,
			.data_length = cpu_to_le32(length),
			.cmd_length = SUNXI_EFEX_CMD_LEN,
			.cmd_package[0] = type,
	};

	const int ret = sunxi_usb_bulk_send(ctx->hdl, ctx->epout, (const char *) &req, sizeof(struct sunxi_usb_request_t));
	if (ret != 0) {
		return EFEX_ERR_USB_TRANSFER;
	}
	return EFEX_ERR_SUCCESS;
}

static int sunxi_read_usb_response_timeout(const struct sunxi_efex_ctx_t *ctx, unsigned int timeout_ms) {
	if (!ctx || !ctx->hdl) {
		return EFEX_ERR_NULL_PTR;
	}

	struct sunxi_usb_response_t resp = {0};

	const int ret = sunxi_usb_bulk_recv_timeout(ctx->hdl, ctx->epin, (char *) &resp, sizeof(resp), timeout_ms);
	if (ret != 0) {
		return ret;
	}

	if (strncmp(resp.magic, SUNXI_USB_RSP_MAGIC, 4) != 0) {
		return EFEX_ERR_INVALID_RESPONSE;
	}

	return resp.status;
}

int sunxi_read_usb_response(const struct sunxi_efex_ctx_t *ctx) {
	return sunxi_read_usb_response_timeout(ctx, DEFAULT_USB_TIMEOUT);
}

int sunxi_usb_write(const struct sunxi_efex_ctx_t *ctx, const void *buf, const size_t len) {
	if (!ctx || !buf) {
		return EFEX_ERR_NULL_PTR;
	}

	int ret = sunxi_send_usb_request(ctx, AW_USB_WRITE, len);
	if (ret != EFEX_ERR_SUCCESS) {
		return ret;
	}

	ret = sunxi_usb_bulk_send(ctx->hdl, ctx->epout, buf, len);
	if (ret != 0) {
		return EFEX_ERR_USB_TRANSFER;
	}

	ret = sunxi_read_usb_response(ctx);
	if (ret != 0) {
		return EFEX_ERR_PROTOCOL;
	}
	return EFEX_ERR_SUCCESS;
}

int sunxi_usb_read(const struct sunxi_efex_ctx_t *ctx, const void *data, const size_t len) {
	if (!ctx || !data) {
		return EFEX_ERR_NULL_PTR;
	}

	int ret = sunxi_send_usb_request(ctx, AW_USB_READ, len);
	if (ret != EFEX_ERR_SUCCESS) {
		return ret;
	}

	ret = sunxi_usb_bulk_recv(ctx->hdl, ctx->epin, (char *) data, len);
	if (ret != 0) {
		return EFEX_ERR_USB_TRANSFER;
	}

	ret = sunxi_read_usb_response(ctx);
	if (ret != 0) {
		return EFEX_ERR_PROTOCOL;
	}
	return EFEX_ERR_SUCCESS;
}

int sunxi_usb_fes_xfer(const struct sunxi_efex_ctx_t *ctx, const enum sunxi_usb_fes_xfer_type_t type,
                       const uint32_t cmd, const char *request_buf, const ssize_t request_len, const char *buf,
                       const ssize_t len) {
	if (!ctx) {
		return EFEX_ERR_NULL_PTR;
	}

	if (ctx->resp.mode != DEVICE_MODE_SRV) {
		return EFEX_ERR_INVALID_PARAM;
	}

	struct sunxi_fes_xfer_t fes_xfer = {
			.cmd = cpu_to_le16((uint16_t) cmd),
			.tag = 0x0,
			.magics = SUNXI_USB_REQ_MAGIC_INT,
	};

	if (request_len > 0 && (size_t)request_len <= sizeof(fes_xfer.buf)) {
		memcpy(fes_xfer.buf, request_buf, request_len);
	}
	uint32_t flags = 0;
	if (cmd == EFEX_CMD_FES_DOWN && request_buf && request_len == sizeof(struct sunxi_fes_trans_t)) {
		struct sunxi_fes_trans_t trans;
		memcpy(&trans, request_buf, sizeof(trans));
		flags = le32_to_cpu(trans.flags);
	}
	const int mbr_commit = cmd == EFEX_CMD_FES_DOWN
		&& (flags & SUNXI_EFEX_DATA_TYPE_MASK) == SUNXI_EFEX_MBR_TAG
		&& (flags & SUNXI_EFEX_TRANS_FINISH_TAG);
	const uint64_t started = sunxi_usb_monotonic_ms();
	if (started == UINT64_MAX) return EFEX_ERR_USB_TRANSFER;
	const char *phase = "command";

	int ret = sunxi_usb_bulk_send(ctx->hdl, ctx->epout, (const char *) &fes_xfer, sizeof(fes_xfer));
	if (ret != 0) {
		goto transfer_failed;
	}

	if (type == FES_XFER_SEND && len > 0) {
		if (!buf) {
			return EFEX_ERR_NULL_PTR;
		}
		phase = "payload_send";
		ret = sunxi_usb_bulk_send(ctx->hdl, ctx->epout, buf, len);
		if (ret != 0) {
			goto transfer_failed;
		}
	} else if (type == FES_XFER_RECV && len > 0) {
		if (!buf) {
			return EFEX_ERR_NULL_PTR;
		}
		phase = "payload_recv";
		ret = sunxi_usb_bulk_recv(ctx->hdl, ctx->epin, (char *) buf, len);
		if (ret != 0) {
			goto transfer_failed;
		}
	}

	/*
	 * The 20-byte FES command uses the "new" device state machine. After its
	 * optional raw data phase, the device sends the 13-byte AWUS CSW directly.
	 * Do not wrap this read in AW_USB_READ: doing so injects a legacy CBW while
	 * the device is in APPS_STATUS (or after it returned to APPS_IDLE), which
	 * produces the deterministic "INVALID direction" errors seen on T153.
	 */
	phase = "status_recv";
	if (mbr_commit) {
		fprintf(stderr, "FES_MBR_COMMIT_WAIT timeout_ms=%u response_bytes=%zu\n",
		        FES_MBR_COMMIT_TIMEOUT_MS, sizeof(struct sunxi_usb_response_t));
	}
	ret = sunxi_read_usb_response_timeout(ctx, mbr_commit ? FES_MBR_COMMIT_TIMEOUT_MS : DEFAULT_USB_TIMEOUT);
	if (ret != 0) {
		/* Keep a positive AWUS CSW status so callers can report the exact
		 * device-side rejection instead of collapsing it into Protocol. */
		goto transfer_failed;
	}
	if (mbr_commit) {
		fprintf(stderr, "FES_MBR_COMMIT_READY elapsed_ms=%llu\n",
		        (unsigned long long) (sunxi_usb_monotonic_ms() - started));
	}

	return EFEX_ERR_SUCCESS;

transfer_failed:
	fprintf(stderr, "FES_TRANSFER_ERROR cmd=0x%04x flags=0x%08x phase=%s bytes=%lld code=%d elapsed_ms=%llu\n",
	        cmd, flags, phase, (long long) len, ret,
	        (unsigned long long) (sunxi_usb_monotonic_ms() - started));
	return ret;
}

void sunxi_usb_hex_dump(const void *buf, size_t len, const char *type) {
#if DEBUG_USB_TRANSFER
	if (!buf) {
		fprintf(stdout, "USB %s len=0\n", type ? type : "");
		fprintf(stdout, "<empty>\n");
		return;
	}

	fprintf(stdout, "USB %s len=%zu\n", type ? type : "", len);

	const unsigned char *p = (const unsigned char *) buf;
	for (size_t j = 0; j < len; j += 16) {
		fprintf(stdout, "%08zx: ", j);
		// hex bytes
		for (size_t i = 0; i < 16; i++) {
			if (j + i < len)
				fprintf(stdout, "%02x ", p[j + i]);
			else
				fprintf(stdout, "   ");
		}
		fputc(' ', stdout);
		// ASCII
		for (size_t i = 0; i < 16; i++) {
			if (j + i >= len)
				fputc(' ', stdout);
			else
				fputc(isprint(p[j + i]) ? p[j + i] : '.', stdout);
		}
		fputc('\n', stdout);
	}
#else
	(void)buf;
	(void)len;
	(void)type;
#endif /* DEBUG_USB_TRANSFER */
}
