#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <ctype.h>

#include "efex-common.h"
#include "efex-protocol.h"
#include "efex-usb.h"
#include "ending.h"
#include "usb_layer.h"

#ifdef _WIN32
#include <setupapi.h>
#include <initguid.h>
#include <usbiodef.h>
#include <windows.h>

#define WINUSB_IO_TIMEOUT_MS 10000
#define WINUSB_CANCEL_GRACE_MS 1000

static int winusb_device_io_control(HANDLE usb_handle, DWORD control_code, void *buffer, DWORD length,
                                    DWORD *transferred, BOOL receive, DWORD timeout_ms) {
	OVERLAPPED *operation = (OVERLAPPED *) calloc(1, sizeof(*operation));
	void *io_buffer = malloc(length);
	if (operation == NULL || io_buffer == NULL) {
		free(operation);
		free(io_buffer);
		return EFEX_ERR_USB_TRANSFER;
	}
	if (!receive) {
		memcpy(io_buffer, buffer, length);
	}
	operation->hEvent = CreateEvent(NULL, TRUE, FALSE, NULL);
	if (operation->hEvent == NULL) {
		free(io_buffer);
		free(operation);
		return EFEX_ERR_USB_TRANSFER;
	}

	*transferred = 0;
	BOOL completed = DeviceIoControl(usb_handle, control_code, NULL, 0, io_buffer, length, NULL, operation);
	if (!completed && GetLastError() != ERROR_IO_PENDING) {
		CloseHandle(operation->hEvent);
		free(io_buffer);
		free(operation);
		return EFEX_ERR_USB_TRANSFER;
	}

	if (!completed) {
		DWORD const wait_result = WaitForSingleObject(operation->hEvent, timeout_ms);
		if (wait_result == WAIT_TIMEOUT) {
			CancelIoEx(usb_handle, operation);
			DWORD const cancel_result = WaitForSingleObject(operation->hEvent, WINUSB_CANCEL_GRACE_MS);
			if (cancel_result == WAIT_OBJECT_0) {
				DWORD ignored = 0;
				GetOverlappedResult(usb_handle, operation, &ignored, FALSE);
				CloseHandle(operation->hEvent);
				free(io_buffer);
				free(operation);
			}
			/*
			 * Some legacy Allwinner Windows drivers do not complete a cancelled
			 * IOCTL. In that case the OVERLAPPED structure and transfer buffer
			 * must remain alive until the Context closes the device handle. Leak
			 * this one timed-out request deliberately instead of blocking forever
			 * or returning with pointers into a dead stack frame.
			 */
			return EFEX_ERR_USB_TIMEOUT;
		}
		if (wait_result != WAIT_OBJECT_0
		    || !GetOverlappedResult(usb_handle, operation, transferred, FALSE)) {
			CloseHandle(operation->hEvent);
			free(io_buffer);
			free(operation);
			return EFEX_ERR_USB_TRANSFER;
		}
	} else if (!GetOverlappedResult(usb_handle, operation, transferred, FALSE)) {
		CloseHandle(operation->hEvent);
		free(io_buffer);
		free(operation);
		return EFEX_ERR_USB_TRANSFER;
	}

	if (receive && *transferred > 0) {
		memcpy(buffer, io_buffer, *transferred);
	}
	CloseHandle(operation->hEvent);
	free(io_buffer);
	free(operation);
	return *transferred > 0 ? EFEX_ERR_SUCCESS : EFEX_ERR_USB_TRANSFER;
}

static int match_vid_pid(const char *device_path) {
	if (!device_path)
		return 0;
	char pattern[64];
	snprintf(pattern, sizeof(pattern), "vid_%04x&pid_%04x", SUNXI_USB_VENDOR, SUNXI_USB_PRODUCT);
	const size_t n = strlen(device_path);

	char *lower = (char *) malloc(n + 1);

	if (!lower)
		return 0;
	for (size_t i = 0; i < n; ++i)
		lower[i] = (char) tolower((unsigned char) device_path[i]);

	lower[n] = '\0';

	const int found = strstr(lower, pattern) != NULL;
	free(lower);
	return found;
}

static char *duplicate_string(const char *src) {
	if (!src) {
		return NULL;
	}

	const size_t len = strlen(src) + 1;
	char *copy = (char *) malloc(len);
	if (!copy) {
		return NULL;
	}
	memcpy(copy, src, len);
	return copy;
}

static int try_read_dword_property(HDEVINFO device_info_set, PSP_DEVINFO_DATA dev_info_data, DWORD property,
                                   DWORD *value) {
	DWORD property_type = 0;
	DWORD required_size = 0;

	return SetupDiGetDeviceRegistryProperty(device_info_set, dev_info_data, property, &property_type, (PBYTE) value,
	                                        sizeof(*value), &required_size)
	       && required_size == sizeof(*value);
}

static int try_read_port_number(HDEVINFO device_info_set, PSP_DEVINFO_DATA dev_info_data, DWORD *port_nr) {
	char buffer[512];
	const char *start = NULL;
	char *end = NULL;
	unsigned long port = 0;

	if (SetupDiGetDeviceRegistryProperty(device_info_set, dev_info_data, SPDRP_LOCATION_INFORMATION, NULL,
	                                     (PBYTE) buffer, sizeof(buffer), NULL)) {
		if (strncmp(buffer, "Port_#", 6) == 0) {
			start = buffer + 6;
			port = strtoul(start, &end, 10);
			if (port > 0 && port <= 0xff && end != start && (*end == '.' || *end == '\0')) {
				*port_nr = (DWORD) port;
				return TRUE;
			}
		}
	}

	if (SetupDiGetDeviceRegistryProperty(device_info_set, dev_info_data, SPDRP_LOCATION_PATHS, NULL, (PBYTE) buffer,
	                                     sizeof(buffer), NULL)) {
		for (char *token = strrchr(buffer, '#'); token != NULL; token = strrchr(buffer, '#')) {
			if (strncmp(token, "#USB(", 5) == 0) {
				start = token + 5;
				port = strtoul(start, &end, 10);
				if (port > 0 && port <= 0xff && end != start && (*end == ')' || *end == '\0')) {
					*port_nr = (DWORD) port;
					return TRUE;
				}
				return FALSE;
			}
			*token = '\0';
		}
	}

	return try_read_dword_property(device_info_set, dev_info_data, SPDRP_ADDRESS, port_nr) && *port_nr <= 0xff;
}

static char *try_read_location_path(HDEVINFO device_info_set, PSP_DEVINFO_DATA dev_info_data) {
	char buffer[1024] = {0};
	DWORD property_type = 0;
	DWORD required_size = 0;
	if (!SetupDiGetDeviceRegistryProperty(device_info_set, dev_info_data, SPDRP_LOCATION_PATHS,
			&property_type, (PBYTE)buffer, sizeof(buffer), &required_size)
		|| required_size == 0 || buffer[0] == '\0') {
		return NULL;
	}
	/* SPDRP_LOCATION_PATHS is REG_MULTI_SZ. The first path is the canonical
	 * physical topology for the active interface and survives FEL/FES
	 * re-enumeration, unlike SetupAPI's enumeration index and DevicePath. */
	return duplicate_string(buffer);
}

static int winusb_bulk_send(void *handle, int ep, const char *buf, ssize_t len) {
	if (!handle || !buf || len <= 0) {
		return EFEX_ERR_NULL_PTR;
	}

	HANDLE const usb_handle = (HANDLE) handle;
	const size_t max_chunk = 128 * 1024;
	DWORD bytes_sent = 0;

	while (len > 0) {
		const size_t chunk = len <= max_chunk ? len : max_chunk;

		sunxi_usb_hex_dump(buf, chunk, "SEND");

		const DWORD dwIoControlCode = CTL_CODE(FILE_DEVICE_UNKNOWN, 0x0807, METHOD_OUT_DIRECT, FILE_ANY_ACCESS);

		int const result = winusb_device_io_control(usb_handle, dwIoControlCode, (LPVOID) buf, (DWORD) chunk,
		                                        &bytes_sent, FALSE, WINUSB_IO_TIMEOUT_MS);

		if (result != EFEX_ERR_SUCCESS) {
			return result;
		}

		len -= bytes_sent;
		buf += bytes_sent;
	}
	return EFEX_ERR_SUCCESS;
}

static int winusb_bulk_recv(void *handle, int ep, char *buf, const ssize_t len) {
	if (!handle || !buf || len <= 0) {
		return EFEX_ERR_NULL_PTR;
	}

	HANDLE const usb_handle = (HANDLE) handle;
	DWORD bytes_received = 0;

	const DWORD dwIoControlCode = CTL_CODE(FILE_DEVICE_UNKNOWN, 0x0808, METHOD_IN_DIRECT, FILE_ANY_ACCESS);

	int const result = winusb_device_io_control(usb_handle, dwIoControlCode, (LPVOID) buf, (DWORD) len,
	                                        &bytes_received, TRUE, WINUSB_IO_TIMEOUT_MS);

	if (result != EFEX_ERR_SUCCESS) {
		return result;
	}

	sunxi_usb_hex_dump(buf, (size_t) bytes_received, "RECV");

	return EFEX_ERR_SUCCESS;
}

static int winusb_bulk_recv_timeout(void *handle, int ep, char *buf, ssize_t len, unsigned int timeout_ms) {
	(void) ep;
	if (!handle || !buf || len <= 0) return EFEX_ERR_NULL_PTR;
	const uint64_t deadline = sunxi_usb_monotonic_ms() + timeout_ms;
	const DWORD control = CTL_CODE(FILE_DEVICE_UNKNOWN, 0x0808, METHOD_IN_DIRECT, FILE_ANY_ACCESS);
	while (len > 0) {
		const uint64_t now = sunxi_usb_monotonic_ms();
		if (now >= deadline) return EFEX_ERR_USB_TIMEOUT;
		DWORD received = 0;
		const int result = winusb_device_io_control((HANDLE) handle, control, buf, (DWORD) len,
		                                          &received, TRUE, (DWORD) (deadline - now));
		if (result != EFEX_ERR_SUCCESS) return result;
		len -= received;
		buf += received;
	}
	return EFEX_ERR_SUCCESS;
}

static int winusb_scan_device(struct sunxi_efex_ctx_t *ctx) {
	if (!ctx) {
		return EFEX_ERR_NULL_PTR;
	}

	SP_DEVICE_INTERFACE_DATA interface_data;
	ULONG index = 0;
	BOOL device_found = FALSE;

	const LPGUID usb_device_guid = (LPGUID) &GUID_DEVINTERFACE_USB_DEVICE;

	const HDEVINFO device_info_set =
			SetupDiGetClassDevs(usb_device_guid, NULL, NULL, DIGCF_PRESENT | DIGCF_DEVICEINTERFACE);

	if (device_info_set == INVALID_HANDLE_VALUE) {
		return EFEX_ERR_USB_INIT;
	}

	BOOL result = TRUE;

	while (result) {
		interface_data.cbSize = sizeof(interface_data);
		result = SetupDiEnumDeviceInterfaces(device_info_set, NULL, usb_device_guid, (ULONG) index, &interface_data);

		if (result) {
			DWORD required_size = 0;
			SetupDiGetDeviceInterfaceDetail(device_info_set, &interface_data, NULL, 0, &required_size, NULL);
			if (required_size == 0) {
				index++;
				continue;
			}

			const PSP_DEVICE_INTERFACE_DETAIL_DATA detail_data =
					(PSP_DEVICE_INTERFACE_DETAIL_DATA) malloc(required_size);
			if (detail_data == NULL) {
				SetupDiDestroyDeviceInfoList(device_info_set);
				return EFEX_ERR_MEMORY;
			}
			detail_data->cbSize = sizeof(SP_DEVICE_INTERFACE_DETAIL_DATA);

			if (SetupDiGetDeviceInterfaceDetail(device_info_set, &interface_data, detail_data, required_size, NULL,
			                                    NULL)) {
				if (match_vid_pid(detail_data->DevicePath)) {
					ctx->dev_name = (char *) malloc(strlen(detail_data->DevicePath) + 1);
					if (ctx->dev_name != NULL) {
						strcpy(ctx->dev_name, detail_data->DevicePath);
						device_found = TRUE;
						free(detail_data);
						break;
					}
					free(detail_data);
					SetupDiDestroyDeviceInfoList(device_info_set);
					return EFEX_ERR_MEMORY;
				}
			}
			free(detail_data);
			index++;
		}
	}

	SetupDiDestroyDeviceInfoList(device_info_set);

	return device_found ? EFEX_ERR_SUCCESS : EFEX_ERR_USB_DEVICE_NOT_FOUND;
}

static int winusb_scan_devices(struct sunxi_scanned_device_t **devices, size_t *count) {
	if (!devices || !count) {
		return EFEX_ERR_NULL_PTR;
	}

	*devices = NULL;
	*count = 0;

	SP_DEVICE_INTERFACE_DATA interface_data;
	ULONG index = 0;
	size_t found_count = 0;

	const LPGUID usb_device_guid = (LPGUID) &GUID_DEVINTERFACE_USB_DEVICE;

	const HDEVINFO device_info_set =
			SetupDiGetClassDevs(usb_device_guid, NULL, NULL, DIGCF_PRESENT | DIGCF_DEVICEINTERFACE);

	if (device_info_set == INVALID_HANDLE_VALUE) {
		return EFEX_ERR_USB_INIT;
	}

	BOOL result = TRUE;
	while (result) {
		interface_data.cbSize = sizeof(interface_data);
		result = SetupDiEnumDeviceInterfaces(device_info_set, NULL, usb_device_guid, (ULONG) index, &interface_data);

		if (result) {
			DWORD required_size = 0;
			SetupDiGetDeviceInterfaceDetail(device_info_set, &interface_data, NULL, 0, &required_size, NULL);
			if (required_size == 0) {
				index++;
				continue;
			}

			const PSP_DEVICE_INTERFACE_DETAIL_DATA detail_data =
					(PSP_DEVICE_INTERFACE_DETAIL_DATA) malloc(required_size);
			if (detail_data == NULL) {
				SetupDiDestroyDeviceInfoList(device_info_set);
				return EFEX_ERR_MEMORY;
			}
			detail_data->cbSize = sizeof(SP_DEVICE_INTERFACE_DETAIL_DATA);

			if (SetupDiGetDeviceInterfaceDetail(device_info_set, &interface_data, detail_data, required_size, NULL,
			                                    NULL)) {
				if (match_vid_pid(detail_data->DevicePath)) {
					found_count++;
				}
			}
			free(detail_data);
			index++;
		}
	}

	if (found_count == 0) {
		SetupDiDestroyDeviceInfoList(device_info_set);
		return EFEX_ERR_USB_DEVICE_NOT_FOUND;
	}

	struct sunxi_scanned_device_t *result_devices = (struct sunxi_scanned_device_t *)malloc(
		sizeof(struct sunxi_scanned_device_t) * found_count);
	if (!result_devices) {
		SetupDiDestroyDeviceInfoList(device_info_set);
		return EFEX_ERR_MEMORY;
	}
	memset(result_devices, 0, sizeof(struct sunxi_scanned_device_t) * found_count);

	index = 0;
	size_t idx = 0;
	result = TRUE;
	while (result && idx < found_count) {
		interface_data.cbSize = sizeof(interface_data);
		result = SetupDiEnumDeviceInterfaces(device_info_set, NULL, usb_device_guid, (ULONG) index, &interface_data);

		if (result) {
			DWORD required_size = 0;
			SetupDiGetDeviceInterfaceDetail(device_info_set, &interface_data, NULL, 0, &required_size, NULL);
			if (required_size == 0) {
				index++;
				continue;
			}

			const PSP_DEVICE_INTERFACE_DETAIL_DATA detail_data =
					(PSP_DEVICE_INTERFACE_DETAIL_DATA) malloc(required_size);
			if (detail_data == NULL) {
				free(result_devices);
				SetupDiDestroyDeviceInfoList(device_info_set);
				return EFEX_ERR_MEMORY;
			}
			detail_data->cbSize = sizeof(SP_DEVICE_INTERFACE_DETAIL_DATA);

			SP_DEVINFO_DATA dev_info_data;
			memset(&dev_info_data, 0, sizeof(dev_info_data));
			dev_info_data.cbSize = sizeof(dev_info_data);
			if (SetupDiGetDeviceInterfaceDetail(device_info_set, &interface_data, detail_data, required_size, NULL,
			                                    &dev_info_data)) {
				if (match_vid_pid(detail_data->DevicePath)) {
					DWORD bus = 0, port = 0;
					try_read_dword_property(device_info_set, &dev_info_data, SPDRP_BUSNUMBER, &bus);
					try_read_port_number(device_info_set, &dev_info_data, &port);
					result_devices[idx].bus = bus > 0xff ? 0 : (uint8_t)bus;
					result_devices[idx].port = port > 0xff ? 0 : (uint8_t)port;
					result_devices[idx].vid = SUNXI_USB_VENDOR;
					result_devices[idx].pid = SUNXI_USB_PRODUCT;
					result_devices[idx].location_path = try_read_location_path(device_info_set, &dev_info_data);
					result_devices[idx].device_path = duplicate_string(detail_data->DevicePath);
					if (!result_devices[idx].device_path) {
						free(detail_data);
						SetupDiDestroyDeviceInfoList(device_info_set);
						sunxi_free_scanned_devices(result_devices, found_count);
						return EFEX_ERR_MEMORY;
					}
					idx++;
				}
			}
			free(detail_data);
			index++;
		}
	}

	SetupDiDestroyDeviceInfoList(device_info_set);

	*devices = result_devices;
	*count = found_count;
	return EFEX_ERR_SUCCESS;
}

static int winusb_scan_device_at(struct sunxi_efex_ctx_t *ctx, uint8_t bus, uint8_t port) {
	if (!ctx) {
		return EFEX_ERR_NULL_PTR;
	}

	SP_DEVICE_INTERFACE_DATA interface_data;
	BOOL device_found = FALSE;

	const LPGUID usb_device_guid = (LPGUID) &GUID_DEVINTERFACE_USB_DEVICE;

	const HDEVINFO device_info_set =
			SetupDiGetClassDevs(usb_device_guid, NULL, NULL, DIGCF_PRESENT | DIGCF_DEVICEINTERFACE);

	if (device_info_set == INVALID_HANDLE_VALUE) {
		return EFEX_ERR_USB_INIT;
	}

	for (ULONG index = 0; !device_found; ++index) {
		interface_data.cbSize = sizeof(interface_data);
		if (!SetupDiEnumDeviceInterfaces(device_info_set, NULL, usb_device_guid, index, &interface_data)) break;
		DWORD required_size = 0;
		SetupDiGetDeviceInterfaceDetail(device_info_set, &interface_data, NULL, 0, &required_size, NULL);
		if (!required_size) continue;
		PSP_DEVICE_INTERFACE_DETAIL_DATA detail_data = (PSP_DEVICE_INTERFACE_DETAIL_DATA)malloc(required_size);
		if (!detail_data) { SetupDiDestroyDeviceInfoList(device_info_set); return EFEX_ERR_MEMORY; }
		SP_DEVINFO_DATA dev_info_data = {0};
		dev_info_data.cbSize = sizeof(dev_info_data);
		detail_data->cbSize = sizeof(SP_DEVICE_INTERFACE_DETAIL_DATA);
		if (SetupDiGetDeviceInterfaceDetail(device_info_set, &interface_data, detail_data, required_size, NULL,
				&dev_info_data) && match_vid_pid(detail_data->DevicePath)) {
			DWORD found_bus = 0, found_port = 0;
			try_read_dword_property(device_info_set, &dev_info_data, SPDRP_BUSNUMBER, &found_bus);
			try_read_port_number(device_info_set, &dev_info_data, &found_port);
			if (found_bus == bus && found_port == port) {
				ctx->dev_name = duplicate_string(detail_data->DevicePath);
				device_found = ctx->dev_name != NULL;
			}
		}
		free(detail_data);
	}
	SetupDiDestroyDeviceInfoList(device_info_set);

	return device_found ? EFEX_ERR_SUCCESS : EFEX_ERR_USB_DEVICE_NOT_FOUND;
}

static int winusb_scan_device_by_location(struct sunxi_efex_ctx_t *ctx, const char *location_path) {
	if (!ctx || !location_path || !location_path[0]) return EFEX_ERR_INVALID_PARAM;
	const LPGUID guid = (LPGUID)&GUID_DEVINTERFACE_USB_DEVICE;
	HDEVINFO set = SetupDiGetClassDevs(guid, NULL, NULL, DIGCF_PRESENT | DIGCF_DEVICEINTERFACE);
	if (set == INVALID_HANDLE_VALUE) return EFEX_ERR_USB_INIT;
	BOOL found = FALSE;
	for (ULONG index = 0; !found; ++index) {
		SP_DEVICE_INTERFACE_DATA interface_data = {0};
		interface_data.cbSize = sizeof(interface_data);
		if (!SetupDiEnumDeviceInterfaces(set, NULL, guid, index, &interface_data)) break;
		DWORD required_size = 0;
		SetupDiGetDeviceInterfaceDetail(set, &interface_data, NULL, 0, &required_size, NULL);
		if (!required_size) continue;
		PSP_DEVICE_INTERFACE_DETAIL_DATA detail = (PSP_DEVICE_INTERFACE_DETAIL_DATA)malloc(required_size);
		if (!detail) { SetupDiDestroyDeviceInfoList(set); return EFEX_ERR_MEMORY; }
		SP_DEVINFO_DATA info = {0};
		info.cbSize = sizeof(info);
		detail->cbSize = sizeof(SP_DEVICE_INTERFACE_DETAIL_DATA);
		if (SetupDiGetDeviceInterfaceDetail(set, &interface_data, detail, required_size, NULL, &info)
				&& match_vid_pid(detail->DevicePath)) {
			char *candidate = try_read_location_path(set, &info);
			if (candidate && _stricmp(candidate, location_path) == 0) {
				ctx->dev_name = duplicate_string(detail->DevicePath);
				found = ctx->dev_name != NULL;
			}
			free(candidate);
		}
		free(detail);
	}
	SetupDiDestroyDeviceInfoList(set);
	return found ? EFEX_ERR_SUCCESS : EFEX_ERR_USB_DEVICE_NOT_FOUND;
}

static int winusb_hotplug_snapshot(struct sunxi_hotplug_device_t **devices, size_t *count) {
	if (!devices || !count) {
		return EFEX_ERR_NULL_PTR;
	}

	*devices = NULL;
	*count = 0;

	SP_DEVICE_INTERFACE_DATA interface_data;
	ULONG index = 0;
	size_t found_count = 0;

	const LPGUID usb_device_guid = (LPGUID) &GUID_DEVINTERFACE_USB_DEVICE;
	const HDEVINFO device_info_set =
			SetupDiGetClassDevs(usb_device_guid, NULL, NULL, DIGCF_PRESENT | DIGCF_DEVICEINTERFACE);

	if (device_info_set == INVALID_HANDLE_VALUE) {
		return EFEX_ERR_USB_INIT;
	}

	BOOL result = TRUE;
	while (result) {
		interface_data.cbSize = sizeof(interface_data);
		result = SetupDiEnumDeviceInterfaces(device_info_set, NULL, usb_device_guid, (ULONG) index, &interface_data);

		if (result) {
			DWORD required_size = 0;
			SetupDiGetDeviceInterfaceDetail(device_info_set, &interface_data, NULL, 0, &required_size, NULL);
			if (required_size == 0) {
				index++;
				continue;
			}

			const PSP_DEVICE_INTERFACE_DETAIL_DATA detail_data =
					(PSP_DEVICE_INTERFACE_DETAIL_DATA) malloc(required_size);
			if (detail_data == NULL) {
				SetupDiDestroyDeviceInfoList(device_info_set);
				return EFEX_ERR_MEMORY;
			}
			SP_DEVINFO_DATA dev_info_data;
			memset(&dev_info_data, 0, sizeof(dev_info_data));
			dev_info_data.cbSize = sizeof(dev_info_data);
			detail_data->cbSize = sizeof(SP_DEVICE_INTERFACE_DETAIL_DATA);

			if (SetupDiGetDeviceInterfaceDetail(device_info_set, &interface_data, detail_data, required_size, NULL,
			                                    &dev_info_data)) {
				if (match_vid_pid(detail_data->DevicePath)) {
					found_count++;
				}
			}
			free(detail_data);
			index++;
		}
	}

	if (found_count == 0) {
		SetupDiDestroyDeviceInfoList(device_info_set);
		return EFEX_ERR_SUCCESS;
	}

	struct sunxi_hotplug_device_t *result_devices = (struct sunxi_hotplug_device_t *) malloc(
		sizeof(struct sunxi_hotplug_device_t) * found_count);
	if (!result_devices) {
		SetupDiDestroyDeviceInfoList(device_info_set);
		return EFEX_ERR_MEMORY;
	}
	memset(result_devices, 0, sizeof(struct sunxi_hotplug_device_t) * found_count);

	index = 0;
	size_t idx = 0;
	result = TRUE;
	while (result && idx < found_count) {
		interface_data.cbSize = sizeof(interface_data);
		result = SetupDiEnumDeviceInterfaces(device_info_set, NULL, usb_device_guid, (ULONG) index, &interface_data);

		if (result) {
			DWORD required_size = 0;
			SetupDiGetDeviceInterfaceDetail(device_info_set, &interface_data, NULL, 0, &required_size, NULL);
			if (required_size == 0) {
				index++;
				continue;
			}

			const PSP_DEVICE_INTERFACE_DETAIL_DATA detail_data =
					(PSP_DEVICE_INTERFACE_DETAIL_DATA) malloc(required_size);
			if (detail_data == NULL) {
				SetupDiDestroyDeviceInfoList(device_info_set);
				free(result_devices);
				return EFEX_ERR_MEMORY;
			}

			SP_DEVINFO_DATA dev_info_data;
			DWORD bus_id = 0;
			DWORD usb_device_id = 0;
			DWORD port = 0;

			memset(&dev_info_data, 0, sizeof(dev_info_data));
			dev_info_data.cbSize = sizeof(dev_info_data);
			detail_data->cbSize = sizeof(SP_DEVICE_INTERFACE_DETAIL_DATA);

			if (SetupDiGetDeviceInterfaceDetail(device_info_set, &interface_data, detail_data, required_size, NULL,
			                                    &dev_info_data)) {
				if (match_vid_pid(detail_data->DevicePath)) {
					try_read_dword_property(device_info_set, &dev_info_data, SPDRP_BUSNUMBER, &bus_id);
					try_read_dword_property(device_info_set, &dev_info_data, SPDRP_ADDRESS, &usb_device_id);
					try_read_port_number(device_info_set, &dev_info_data, &port);

					result_devices[idx].vid = SUNXI_USB_VENDOR;
					result_devices[idx].pid = SUNXI_USB_PRODUCT;
					result_devices[idx].bus_id = bus_id;
					result_devices[idx].usb_device_id = usb_device_id;
					result_devices[idx].port = port > 0xff ? 0 : (uint8_t) port;
					result_devices[idx].device_path = duplicate_string(detail_data->DevicePath);
					if (detail_data->DevicePath && result_devices[idx].device_path == NULL) {
						free(detail_data);
						SetupDiDestroyDeviceInfoList(device_info_set);
						for (size_t j = 0; j < idx; ++j) {
							free(result_devices[j].device_path);
						}
						free(result_devices);
						return EFEX_ERR_MEMORY;
					}
					idx++;
				}
			}
			free(detail_data);
			index++;
		}
	}

	SetupDiDestroyDeviceInfoList(device_info_set);

	*devices = result_devices;
	*count = found_count;
	return EFEX_ERR_SUCCESS;
}

static int winusb_init(struct sunxi_efex_ctx_t *ctx) {
	if (!ctx || !ctx->dev_name) {
		return EFEX_ERR_NULL_PTR;
	}

	ctx->hdl = CreateFile(ctx->dev_name, GENERIC_WRITE | GENERIC_READ, FILE_SHARE_WRITE | FILE_SHARE_READ, NULL,
	                      OPEN_EXISTING, FILE_FLAG_OVERLAPPED, NULL);

	if (ctx->hdl == INVALID_HANDLE_VALUE) {
		return EFEX_ERR_USB_OPEN;
	}

	return EFEX_ERR_SUCCESS;
}

static int winusb_exit(struct sunxi_efex_ctx_t *ctx) {
	if (!ctx) {
		return EFEX_ERR_NULL_PTR;
	}

	if (ctx->hdl != NULL && (HANDLE) ctx->hdl != INVALID_HANDLE_VALUE) {
		CloseHandle((HANDLE) ctx->hdl);
		ctx->hdl = NULL;
	}
	if (ctx->dev_name != NULL) {
		free(ctx->dev_name);
		ctx->dev_name = NULL;
	}

	return EFEX_ERR_SUCCESS;
}

const struct usb_backend_ops usb_winusb_ops = {
	.bulk_send = winusb_bulk_send,
	.bulk_recv = winusb_bulk_recv,
	.bulk_recv_timeout = winusb_bulk_recv_timeout,
	.scan_device = winusb_scan_device,
	.scan_device_at = winusb_scan_device_at,
	.scan_device_by_location = winusb_scan_device_by_location,
	.scan_devices = winusb_scan_devices,
	.hotplug_snapshot = winusb_hotplug_snapshot,
	.init = winusb_init,
	.exit = winusb_exit,
};

#endif // _WIN32
