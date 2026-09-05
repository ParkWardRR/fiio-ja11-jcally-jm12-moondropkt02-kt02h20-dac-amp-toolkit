// e3_interface_seize.c — experiment E3 from docs/MACOS-NATIVE.md §4.1.
//
// WHAT THIS TRIES
// ---------------
// If E1/E2 fail — i.e. IOHIDDeviceSetReport really does put output reports on the control pipe
// the firmware ignores — the remaining native option is to bypass IOHIDFamily entirely and
// write to the interrupt-OUT endpoint (0x03) ourselves.
//
// libusb cannot do this on Darwin: its claim path calls USBInterfaceOpen, which fails with
// kIOReturnExclusiveAccess once IOHIDFamily has matched, and there is no detach_kernel_driver
// on macOS. But IOKit exposes USBDeviceOpenSeize / USBInterfaceOpenSeize, which *take* the
// device from the existing client. That needs no kext and no DriverKit entitlement — so unlike
// a DEXT (E5) it stays compatible with shipping an unsigned binary.
//
// OPEN QUESTION: whether current macOS still permits seizing an interface from IOHIDFamily.
// This program answers it. Expect to need sudo.
//
// SAFETY: sends exactly the 10-byte unlock report. Recoverable — power-cycle to undo.
//
// BUILD:  make e3
// RUN:    sudo ./e3_interface_seize          # dry run: describe the interface, send nothing
//         sudo ./e3_interface_seize --send
//
// STATUS: UNTESTED against hardware. Compile-checked only. The IOUSBLib plug-in dance is
// fiddly; expect fixups.

#include <CoreFoundation/CoreFoundation.h>
#include <IOKit/IOCFPlugIn.h>
#include <IOKit/IOKitLib.h>
#include <IOKit/usb/IOUSBLib.h>
#include <stdbool.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

#define VID_KTMICRO 0x31b2
#define VID_FIIO    0x2972
#define BOOT_VID    0x8888
#define BOOT_PID    0xcdc0

#define USB_CLASS_HID 3
// Report 0x54 + "12345678" + NUL, exactly as it goes on the wire (docs/PROTOCOL.md).
static const uint8_t kUnlock[] = {0x54, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x00};

static bool bootloader_present(void) {
    CFMutableDictionaryRef m = IOServiceMatching("IOUSBHostDevice");
    if (!m) return false;
    int vid = BOOT_VID, pid = BOOT_PID;
    CFNumberRef v = CFNumberCreate(NULL, kCFNumberIntType, &vid);
    CFNumberRef p = CFNumberCreate(NULL, kCFNumberIntType, &pid);
    CFDictionarySetValue(m, CFSTR("idVendor"), v);
    CFDictionarySetValue(m, CFSTR("idProduct"), p);
    CFRelease(v);
    CFRelease(p);
    io_service_t svc = IOServiceGetMatchingService(kIOMainPortDefault, m);
    if (svc) { IOObjectRelease(svc); return true; }
    return false;
}

// Get an IOUSBDeviceInterface for the first matching dongle.
static IOUSBDeviceInterface **find_device(void) {
    CFMutableDictionaryRef match = IOServiceMatching(kIOUSBDeviceClassName);
    if (!match) return NULL;

    io_iterator_t it = 0;
    if (IOServiceGetMatchingServices(kIOMainPortDefault, match, &it) != kIOReturnSuccess)
        return NULL;

    IOUSBDeviceInterface **found = NULL;
    io_service_t svc;
    while ((svc = IOIteratorNext(it))) {
        IOCFPlugInInterface **plugin = NULL;
        SInt32 score = 0;
        kern_return_t kr = IOCreatePlugInInterfaceForService(
            svc, kIOUSBDeviceUserClientTypeID, kIOCFPlugInInterfaceID, &plugin, &score);
        IOObjectRelease(svc);
        if (kr != kIOReturnSuccess || !plugin) continue;

        IOUSBDeviceInterface **dev = NULL;
        HRESULT hr = (*plugin)->QueryInterface(
            plugin, CFUUIDGetUUIDBytes(kIOUSBDeviceInterfaceID), (LPVOID *)&dev);
        (*plugin)->Release(plugin);
        if (hr || !dev) continue;

        UInt16 vid = 0, pid = 0;
        (*dev)->GetDeviceVendor(dev, &vid);
        (*dev)->GetDeviceProduct(dev, &pid);
        if (vid == VID_KTMICRO || vid == VID_FIIO) {
            printf("found USB device %04x:%04x\n", vid, pid);
            found = dev;
            break;
        }
        (*dev)->Release(dev);
    }
    IOObjectRelease(it);
    return found;
}

int main(int argc, char **argv) {
    bool send = (argc > 1 && !strcmp(argv[1], "--send"));

    if (bootloader_present()) {
        printf("NOTE: 8888:cdc0 is already present — power-cycle the dongle first.\n");
        return 1;
    }
    if (geteuid() != 0)
        printf("warning: not running as root — seizing an interface will very likely fail.\n");

    IOUSBDeviceInterface **dev = find_device();
    if (!dev) { fprintf(stderr, "no KTMicro/FiiO USB device found\n"); return 1; }

    // Seize, not Open: the point of this experiment is taking the device from IOHIDFamily.
    IOReturn r = (*dev)->USBDeviceOpenSeize(dev);
    if (r != kIOReturnSuccess) {
        fprintf(stderr, "USBDeviceOpenSeize: 0x%08x — try sudo\n", r);
        (*dev)->Release(dev);
        return 1;
    }

    IOUSBFindInterfaceRequest req = {
        .bInterfaceClass = kIOUSBFindInterfaceDontCare,
        .bInterfaceSubClass = kIOUSBFindInterfaceDontCare,
        .bInterfaceProtocol = kIOUSBFindInterfaceDontCare,
        .bAlternateSetting = kIOUSBFindInterfaceDontCare,
    };
    io_iterator_t it = 0;
    if ((*dev)->CreateInterfaceIterator(dev, &req, &it) != kIOReturnSuccess) {
        fprintf(stderr, "CreateInterfaceIterator failed\n");
        (*dev)->USBDeviceClose(dev);
        (*dev)->Release(dev);
        return 1;
    }

    int rc = 1;
    io_service_t usb_iface;
    while ((usb_iface = IOIteratorNext(it))) {
        IOCFPlugInInterface **plugin = NULL;
        SInt32 score = 0;
        kern_return_t kr = IOCreatePlugInInterfaceForService(
            usb_iface, kIOUSBInterfaceUserClientTypeID, kIOCFPlugInInterfaceID, &plugin, &score);
        IOObjectRelease(usb_iface);
        if (kr != kIOReturnSuccess || !plugin) continue;

        IOUSBInterfaceInterface **intf = NULL;
        HRESULT hr = (*plugin)->QueryInterface(
            plugin, CFUUIDGetUUIDBytes(kIOUSBInterfaceInterfaceID), (LPVOID *)&intf);
        (*plugin)->Release(plugin);
        if (hr || !intf) continue;

        UInt8 cls = 0, num = 0;
        (*intf)->GetInterfaceClass(intf, &cls);
        (*intf)->GetInterfaceNumber(intf, &num);
        if (cls != USB_CLASS_HID) { (*intf)->Release(intf); continue; }
        printf("interface %u: class %u (HID)\n", num, cls);

        // THE experiment: can we take this interface away from IOHIDFamily?
        IOReturn ir = (*intf)->USBInterfaceOpenSeize(intf);
        if (ir != kIOReturnSuccess) {
            printf("  USBInterfaceOpenSeize -> 0x%08x  ✗ (0xe00002c5 = exclusive access)\n", ir);
            (*intf)->Release(intf);
            continue;
        }
        printf("  USBInterfaceOpenSeize -> success ✓  (macOS DOES allow seizing from IOHIDFamily)\n");

        UInt8 npipes = 0;
        (*intf)->GetNumEndpoints(intf, &npipes);
        int out_pipe = -1;
        for (UInt8 p = 1; p <= npipes; p++) {
            UInt8 dir = 0, pnum = 0, ptype = 0, interval = 0;
            UInt16 maxpkt = 0;
            if ((*intf)->GetPipeProperties(intf, p, &dir, &pnum, &ptype, &maxpkt, &interval) !=
                kIOReturnSuccess)
                continue;
            printf("    pipe %u: dir=%s type=%u maxPacket=%u\n", p,
                   dir == kUSBOut ? "OUT" : (dir == kUSBIn ? "IN" : "?"), ptype, maxpkt);
            if (dir == kUSBOut && ptype == kUSBInterrupt && out_pipe < 0) out_pipe = p;
        }

        if (out_pipe < 0) {
            printf("  no interrupt-OUT pipe on this interface\n");
        } else if (!send) {
            printf("  [dry run] would WritePipe %zu bytes to pipe %d. Re-run with --send.\n",
                   sizeof kUnlock, out_pipe);
            rc = 0;
        } else {
            // The whole point: the interrupt-OUT endpoint the firmware actually reads,
            // rather than the control pipe IOHIDDeviceSetReport uses.
            IOReturn wr = (*intf)->WritePipe(intf, (UInt8)out_pipe, (void *)kUnlock,
                                             (UInt32)sizeof kUnlock);
            printf("  WritePipe -> 0x%08x %s\n", wr, wr == kIOReturnSuccess ? "(sent)" : "(FAILED)");

            printf("\nwatching for the 8888:cdc0 bootloader (5s)…\n");
            for (int i = 0; i < 25; i++) {
                if (bootloader_present()) {
                    printf("\n*** PASS: bootloader appeared. Native macOS unlock works via E3. ***\n");
                    printf("    Port this into ktflash as a macOS-only unlock transport.\n");
                    printf("    Note whether sudo was required — that belongs in the docs.\n");
                    rc = 0;
                    break;
                }
                usleep(200000);
            }
            if (rc != 0) printf("\n*** FAIL: no re-enumeration after a successful seize+write. ***\n");
        }

        (*intf)->USBInterfaceClose(intf);
        (*intf)->Release(intf);
        break;
    }
    IOObjectRelease(it);

    (*dev)->USBDeviceClose(dev);
    (*dev)->Release(dev);
    return rc;
}
