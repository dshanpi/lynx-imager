#include <ctype.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <libusb.h>

#include "compiler.h"
#include "efex-common.h"
#include "efex-protocol.h"
#include "efex-usb.h"
#include "ending.h"
#include "usb_layer.h"

static int libusb_scan_device_at(struct sunxi_efex_ctx_t *ctx, uint8_t bus, uint8_t port);

static int format_libusb_location(libusb_device *device, char *buffer, size_t buffer_len) {
	uint8_t ports[8];
	if (!device || !buffer || buffer_len == 0) return -1;
	const int depth = libusb_get_port_numbers(device, ports, (int)(sizeof(ports) / sizeof(ports[0])));
	if (depth <= 0) return -1;
	int written = snprintf(buffer, buffer_len, "libusb:%u:",
		(unsigned)libusb_get_bus_number(device));
	if (written < 0 || (size_t)written >= buffer_len) return -1;
	for (int i = 0; i < depth; i++) {
		const int item = snprintf(buffer + written, buffer_len - (size_t)written,
			"%s%u", i == 0 ? "" : ".", (unsigned)ports[i]);
		if (item < 0 || (size_t)item >= buffer_len - (size_t)written) return -1;
		written += item;
	}
	return 0;
}

static int libusb_location_matches(libusb_device *device, const char *expected) {
	char current[128];
	if (format_libusb_location(device, current, sizeof(current)) != 0) return 0;
	if (strcmp(current, expected) == 0) return 1;

	/* Compatibility with LYNX <=0.6.10, which stored only bus and leaf port. */
	unsigned int bus = 0, leaf = 0;
	char trailing = '\0';
	if (sscanf(expected, "libusb:%u:%u%c", &bus, &leaf, &trailing) != 2) return 0;
	uint8_t ports[8];
	const int depth = libusb_get_port_numbers(device, ports, (int)(sizeof(ports) / sizeof(ports[0])));
	return depth > 0 && libusb_get_bus_number(device) == bus && ports[depth - 1] == leaf;
}

static int libusb_bulk_send(void *handle, int ep, const char *buf, ssize_t len) {
	if (!handle || !buf || len <= 0) {
		return EFEX_ERR_NULL_PTR;
	}

	libusb_device_handle *hdl = (libusb_device_handle *) handle;
	const size_t max_chunk = 128 * 1024;
	int bytes;

	while (len > 0) {
		const size_t chunk = (size_t)len < max_chunk ? (size_t)len : max_chunk;

		sunxi_usb_hex_dump(buf, chunk, "SEND");

		const int r = libusb_bulk_transfer(hdl, ep, (void *) buf, (int) chunk, &bytes, DEFAULT_USB_TIMEOUT);
		if (r == LIBUSB_ERROR_TIMEOUT) {
			return EFEX_ERR_USB_TIMEOUT;
		}
		if (r != 0 || bytes <= 0) {
			return EFEX_ERR_USB_TRANSFER;
		}
		len -= bytes;
		buf += bytes;
	}
	return EFEX_ERR_SUCCESS;
}

static int libusb_bulk_recv_timeout(void *handle, int ep, char *buf, ssize_t len, unsigned int timeout_ms) {
	if (!handle || !buf || len <= 0) {
		return EFEX_ERR_NULL_PTR;
	}

	libusb_device_handle *hdl = (libusb_device_handle *) handle;
	const uint64_t started = sunxi_usb_monotonic_ms();
	if (started == UINT64_MAX) return EFEX_ERR_USB_TRANSFER;
	const uint64_t deadline = started + timeout_ms;
	const ssize_t requested = len;

	while (len > 0) {
		const uint64_t now = sunxi_usb_monotonic_ms();
		if (now >= deadline) return EFEX_ERR_USB_TIMEOUT;
		int bytes = 0;
		const int r = libusb_bulk_transfer(hdl, ep, (uint8_t *) buf, (int) len, &bytes,
		                                  (unsigned int) (deadline - now));
		if (r != 0 || bytes <= 0) {
			fprintf(stderr, "FES_USB_RECV_ERROR endpoint=0x%02x requested=%lld received=%lld libusb=%d timeout_ms=%u elapsed_ms=%llu\n",
			        ep, (long long) requested, (long long) (requested - len + bytes), r, timeout_ms,
			        (unsigned long long) (sunxi_usb_monotonic_ms() - started));
			return r == LIBUSB_ERROR_TIMEOUT ? EFEX_ERR_USB_TIMEOUT : EFEX_ERR_USB_TRANSFER;
		}

		sunxi_usb_hex_dump(buf, (size_t) bytes, "RECV");

		len -= bytes;
		buf += bytes;
	}
	return EFEX_ERR_SUCCESS;
}

static int libusb_bulk_recv(void *handle, int ep, char *buf, ssize_t len) {
	return libusb_bulk_recv_timeout(handle, ep, buf, len, DEFAULT_USB_TIMEOUT);
}

static int libusb_open_error_to_efex(int rc) {
	if (rc == LIBUSB_ERROR_NOT_SUPPORTED || rc == LIBUSB_ERROR_NOT_FOUND) {
		return EFEX_ERR_USB_WRONG_DRIVER;
	}
	if (rc == LIBUSB_ERROR_ACCESS) {
		return EFEX_ERR_USB_OPEN;
	}
	return EFEX_ERR_USB_INIT;
}

static int libusb_scan_device(struct sunxi_efex_ctx_t *ctx) {
	if (!ctx) {
		return EFEX_ERR_NULL_PTR;
	}

	libusb_device **list = NULL;
	libusb_context *context = NULL;

	libusb_init(&context);
	ctx->usb_context = context;
	const size_t count = libusb_get_device_list(context, &list);
	for (size_t i = 0; i < count; i++) {
		libusb_device *device = list[i];
		struct libusb_device_descriptor desc;
		if (libusb_get_device_descriptor(device, &desc) != 0) {
			return EFEX_ERR_USB_DEVICE_NOT_FOUND;
		}
		if (desc.idVendor == SUNXI_USB_VENDOR && desc.idProduct == SUNXI_USB_PRODUCT) {
			libusb_device_handle *libusb_hdl = NULL;
			int rc = libusb_open(device, &libusb_hdl);
			if (rc != 0) {
				fprintf(stderr, "ERROR: Can't connect to device: %s (rc=%d)\r\n",
					libusb_strerror(rc), rc);
				libusb_free_device_list(list, 1);
				return libusb_open_error_to_efex(rc);
			}
			ctx->hdl = libusb_hdl;
			libusb_free_device_list(list, 1);
			return EFEX_ERR_SUCCESS;
		}
	}
	libusb_free_device_list(list, 1);
	return EFEX_ERR_USB_DEVICE_NOT_FOUND;
}

static int libusb_scan_devices(struct sunxi_scanned_device_t **devices, size_t *count) {
	if (!devices || !count) {
		return EFEX_ERR_NULL_PTR;
	}

	*devices = NULL;
	*count = 0;

	libusb_device **list = NULL;
	libusb_context *context = NULL;

	int r = libusb_init(&context);
	if (r < 0) {
		return EFEX_ERR_USB_INIT;
	}

	const ssize_t device_count = libusb_get_device_list(context, &list);
	if (device_count < 0) {
		libusb_exit(context);
		return EFEX_ERR_USB_DEVICE_NOT_FOUND;
	}

	size_t found_count = 0;
	for (ssize_t i = 0; i < device_count; i++) {
		libusb_device *device = list[i];
		struct libusb_device_descriptor desc;
		if (libusb_get_device_descriptor(device, &desc) != 0) {
			continue;
		}
		if (desc.idVendor == SUNXI_USB_VENDOR && desc.idProduct == SUNXI_USB_PRODUCT) {
			found_count++;
		}
	}

	if (found_count == 0) {
		libusb_free_device_list(list, 1);
		libusb_exit(context);
		return EFEX_ERR_USB_DEVICE_NOT_FOUND;
	}

	struct sunxi_scanned_device_t *result = (struct sunxi_scanned_device_t *)malloc(
		sizeof(struct sunxi_scanned_device_t) * found_count);
	if (!result) {
		libusb_free_device_list(list, 1);
		libusb_exit(context);
		return EFEX_ERR_MEMORY;
	}
	memset(result, 0, sizeof(struct sunxi_scanned_device_t) * found_count);

	size_t idx = 0;
	for (ssize_t i = 0; i < device_count && idx < found_count; i++) {
		libusb_device *device = list[i];
		struct libusb_device_descriptor desc;
		if (libusb_get_device_descriptor(device, &desc) != 0) {
			continue;
		}
		if (desc.idVendor == SUNXI_USB_VENDOR && desc.idProduct == SUNXI_USB_PRODUCT) {
			result[idx].bus = libusb_get_bus_number(device);
			result[idx].port = libusb_get_port_number(device);
			result[idx].vid = desc.idVendor;
			result[idx].pid = desc.idProduct;
			char location[128];
			if (format_libusb_location(device, location, sizeof(location)) == 0)
				result[idx].location_path = strdup(location);
			idx++;
		}
	}

	libusb_free_device_list(list, 1);
	libusb_exit(context);

	*devices = result;
	*count = found_count;
	return EFEX_ERR_SUCCESS;
}

static int libusb_scan_device_by_location(struct sunxi_efex_ctx_t *ctx, const char *location_path) {
	if (!ctx || !location_path) return EFEX_ERR_NULL_PTR;
	if (strncmp(location_path, "libusb:", 7) != 0) return EFEX_ERR_INVALID_PARAM;

	libusb_device **list = NULL;
	libusb_context *context = NULL;
	if (libusb_init(&context) < 0) return EFEX_ERR_USB_INIT;
	ctx->usb_context = context;
	const ssize_t count = libusb_get_device_list(context, &list);
	if (count < 0) return EFEX_ERR_USB_DEVICE_NOT_FOUND;
	libusb_device *matched = NULL;
	for (ssize_t i = 0; i < count; i++) {
		struct libusb_device_descriptor desc;
		if (libusb_get_device_descriptor(list[i], &desc) != 0
			|| desc.idVendor != SUNXI_USB_VENDOR || desc.idProduct != SUNXI_USB_PRODUCT
			|| !libusb_location_matches(list[i], location_path)) continue;
		if (matched) {
			libusb_free_device_list(list, 1);
			return EFEX_ERR_INVALID_PARAM;
		}
		matched = list[i];
	}
	if (!matched) {
		libusb_free_device_list(list, 1);
		return EFEX_ERR_USB_DEVICE_NOT_FOUND;
	}
	libusb_device_handle *handle = NULL;
	const int rc = libusb_open(matched, &handle);
	libusb_free_device_list(list, 1);
	if (rc != 0) return libusb_open_error_to_efex(rc);
	ctx->hdl = handle;
	return EFEX_ERR_SUCCESS;
}

static int libusb_hotplug_snapshot(struct sunxi_hotplug_device_t **devices, size_t *count) {
	if (!devices || !count) {
		return EFEX_ERR_NULL_PTR;
	}

	*devices = NULL;
	*count = 0;

	libusb_device **list = NULL;
	libusb_context *context = NULL;

	int r = libusb_init(&context);
	if (r < 0) {
		return EFEX_ERR_USB_INIT;
	}

	const ssize_t device_count = libusb_get_device_list(context, &list);
	if (device_count < 0) {
		libusb_exit(context);
		return EFEX_ERR_USB_DEVICE_NOT_FOUND;
	}

	size_t found_count = 0;
	for (ssize_t i = 0; i < device_count; i++) {
		libusb_device *device = list[i];
		struct libusb_device_descriptor desc;
		if (libusb_get_device_descriptor(device, &desc) != 0) {
			continue;
		}
		if (desc.idVendor == SUNXI_USB_VENDOR && desc.idProduct == SUNXI_USB_PRODUCT) {
			found_count++;
		}
	}

	if (found_count == 0) {
		libusb_free_device_list(list, 1);
		libusb_exit(context);
		return EFEX_ERR_SUCCESS;
	}

	struct sunxi_hotplug_device_t *result = (struct sunxi_hotplug_device_t *)malloc(
		sizeof(struct sunxi_hotplug_device_t) * found_count);
	if (!result) {
		libusb_free_device_list(list, 1);
		libusb_exit(context);
		return EFEX_ERR_MEMORY;
	}
	memset(result, 0, sizeof(struct sunxi_hotplug_device_t) * found_count);

	size_t idx = 0;
	for (ssize_t i = 0; i < device_count && idx < found_count; i++) {
		libusb_device *device = list[i];
		struct libusb_device_descriptor desc;
		if (libusb_get_device_descriptor(device, &desc) != 0) {
			continue;
		}
		if (desc.idVendor == SUNXI_USB_VENDOR && desc.idProduct == SUNXI_USB_PRODUCT) {
			result[idx].vid = desc.idVendor;
			result[idx].pid = desc.idProduct;
			result[idx].bus_id = libusb_get_bus_number(device);
			result[idx].usb_device_id = libusb_get_device_address(device);
			result[idx].port = libusb_get_port_number(device);
			/* Preserve the complete physical hub path across USB re-enumeration. */
			{
				char buf[128];
				if (format_libusb_location(device, buf, sizeof(buf)) == 0)
					result[idx].device_path = strdup(buf);
			}
			idx++;
		}
	}

	libusb_free_device_list(list, 1);
	libusb_exit(context);

	*devices = result;
	*count = found_count;
	return EFEX_ERR_SUCCESS;
}

static int libusb_scan_device_at(struct sunxi_efex_ctx_t *ctx, uint8_t bus, uint8_t port) {
	if (!ctx) {
		return EFEX_ERR_NULL_PTR;
	}

	libusb_device **list = NULL;
	libusb_context *context = NULL;

	libusb_init(&context);
	ctx->usb_context = context;
	const size_t count = libusb_get_device_list(context, &list);
	for (size_t i = 0; i < count; i++) {
		libusb_device *device = list[i];
		struct libusb_device_descriptor desc;
		if (libusb_get_device_descriptor(device, &desc) != 0) {
			continue;
		}
		if (desc.idVendor == SUNXI_USB_VENDOR && desc.idProduct == SUNXI_USB_PRODUCT) {
			uint8_t dev_bus = libusb_get_bus_number(device);
			uint8_t dev_port = libusb_get_port_number(device);
			if (dev_bus == bus && dev_port == port) {
				libusb_device_handle *libusb_hdl = NULL;
				int rc = libusb_open(device, &libusb_hdl);
				if (rc != 0) {
					fprintf(stderr, "ERROR: Can't connect to device at bus=%d port=%d: %s (rc=%d)\r\n",
						bus, port, libusb_strerror(rc), rc);
					libusb_free_device_list(list, 1);
					return libusb_open_error_to_efex(rc);
				}
				ctx->hdl = libusb_hdl;
				libusb_free_device_list(list, 1);
				return EFEX_ERR_SUCCESS;
			}
		}
	}
	libusb_free_device_list(list, 1);
	return EFEX_ERR_USB_DEVICE_NOT_FOUND;
}

static int libusb_backend_init(struct sunxi_efex_ctx_t *ctx) {
	if (ctx && ctx->hdl) {
		libusb_device_handle *libusb_hdl = (libusb_device_handle *) ctx->hdl;
		struct libusb_config_descriptor *config;

		if (libusb_kernel_driver_active(libusb_hdl, 0))
			libusb_detach_kernel_driver(libusb_hdl, 0);

		if (libusb_claim_interface(libusb_hdl, 0) == 0) {
			if (libusb_get_active_config_descriptor(libusb_get_device(libusb_hdl), &config) == 0) {
				for (int if_idx = 0; if_idx < config->bNumInterfaces; if_idx++) {
					const struct libusb_interface *iface = config->interface + if_idx;
					for (int set_idx = 0; set_idx < iface->num_altsetting; set_idx++) {
						const struct libusb_interface_descriptor *setting = iface->altsetting + set_idx;
						for (int ep_idx = 0; ep_idx < setting->bNumEndpoints; ep_idx++) {
							const struct libusb_endpoint_descriptor *ep = setting->endpoint + ep_idx;
							if ((ep->bmAttributes & LIBUSB_TRANSFER_TYPE_MASK) != LIBUSB_TRANSFER_TYPE_BULK)
								continue;
							if ((ep->bEndpointAddress & LIBUSB_ENDPOINT_DIR_MASK) == LIBUSB_ENDPOINT_IN)
								ctx->epin = ep->bEndpointAddress;
							else
								ctx->epout = ep->bEndpointAddress;
						}
					}
				}
				libusb_free_config_descriptor(config);
				return EFEX_ERR_SUCCESS;
			}
		}
	}
	return EFEX_ERR_USB_INIT;
}

static int libusb_backend_exit(struct sunxi_efex_ctx_t *ctx) {
	if (!ctx) {
		return EFEX_ERR_NULL_PTR;
	}

	if (ctx->hdl) {
		libusb_device_handle *libusb_hdl = (libusb_device_handle *) ctx->hdl;
		libusb_release_interface(libusb_hdl, 0);
		libusb_close(libusb_hdl);
		ctx->hdl = NULL;
	}

	if (ctx->usb_context) {
		libusb_exit((libusb_context *) ctx->usb_context);
		ctx->usb_context = NULL;
	}

	return EFEX_ERR_SUCCESS;
}

const struct usb_backend_ops usb_libusb_ops = {
	.bulk_send = libusb_bulk_send,
	.bulk_recv = libusb_bulk_recv,
	.bulk_recv_timeout = libusb_bulk_recv_timeout,
	.scan_device = libusb_scan_device,
	.scan_device_at = libusb_scan_device_at,
	.scan_device_by_location = libusb_scan_device_by_location,
	.scan_devices = libusb_scan_devices,
	.hotplug_snapshot = libusb_hotplug_snapshot,
	.init = libusb_backend_init,
	.exit = libusb_backend_exit,
};
