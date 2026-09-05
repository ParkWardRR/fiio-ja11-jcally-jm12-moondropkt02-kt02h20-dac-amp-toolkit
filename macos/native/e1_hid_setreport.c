// e1_hid_setreport.c — experiment E1/E2 from docs/MACOS-NATIVE.md §4.1.
//
// QUESTION THIS ANSWERS
// --------------------
// Two files in this repo contradict each other about macOS:
//
//   docs/PROTOCOL.md:            "IOHIDDeviceSetReport sends output reports down the *control*
//                                 pipe, which the firmware ignores (it reads interrupt-OUT)."
//   docs/dongle-investigation.md: "Open (shared), GET_REPORT, and SetReport all work natively
//                                 via IOHIDManager — no Windows needed for the transport."
//
// Both can be literally true (the call returns success; the device never acts). But only one
// can be true about *where the bytes land*, and the whole native-macOS plan hinges on it. The
// investigation was also run against a KZ C04 (31b2:0313), not a JA11.
//
// PASS/FAIL IS UNAMBIGUOUS: the dongle re-enumerates as 8888:cdc0, or it does not.
//
// SAFETY: `unlock` is recoverable. It reboots the dongle into its CDC bootloader; a
// power-cycle (unplug/replug) returns it to normal mode. It does NOT erase anything. This
// program sends exactly one report and never writes flash.
//
// BUILD:  make e1
// RUN:    ./e1_hid_setreport                 # dry run: enumerate and describe, send nothing
//         ./e1_hid_setreport --send          # send the unlock report
//         ./e1_hid_setreport --send --seize  # E2: take exclusive ownership from IOHIDFamily
//         ./e1_hid_setreport --send --no-id-prefix   # buffer WITHOUT the leading report ID
//
// STATUS: UNTESTED against hardware. Compile-checked only.

#include <CoreFoundation/CoreFoundation.h>
#include <IOKit/IOKitLib.h>
#include <IOKit/hid/IOHIDManager.h>
#include <stdbool.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

// Runtime-mode dongles we might find. (docs/PROTOCOL.md, docs/COMPATIBILITY.md)
#define VID_KTMICRO 0x31b2
#define VID_FIIO    0x2972
// The vendor HID collection carrying the 0x4B/0x54 command reports.
#define USAGE_PAGE_VENDOR 0xff01
// Post-unlock bootloader — the thing we are watching for.
#define BOOT_VID 0x8888
#define BOOT_PID 0xcdc0

// Report 0x54 + "12345678" + NUL — on the wire: 54 31 32 33 34 35 36 37 38 00
#define UNLOCK_REPORT_ID 0x54
static const uint8_t kUnlockPayload[] = {0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x00};

static long cf_num(IOHIDDeviceRef dev, CFStringRef key) {
    CFTypeRef v = IOHIDDeviceGetProperty(dev, key);
    long out = -1;
    if (v && CFGetTypeID(v) == CFNumberGetTypeID()) CFNumberGetValue(v, kCFNumberLongType, &out);
    return out;
}

static void cf_str(IOHIDDeviceRef dev, CFStringRef key, char *buf, size_t n) {
    buf[0] = '\0';
    CFTypeRef v = IOHIDDeviceGetProperty(dev, key);
    if (v && CFGetTypeID(v) == CFStringGetTypeID())
        CFStringGetCString(v, buf, (CFIndex)n, kCFStringEncodingUTF8);
}

// Is the 8888:cdc0 bootloader present right now?
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
    io_service_t svc = IOServiceGetMatchingService(kIOMainPortDefault, m); // consumes `m`
    if (svc) {
        IOObjectRelease(svc);
        return true;
    }
    return false;
}

int main(int argc, char **argv) {
    bool send = false, seize = false, id_prefix = true;
    for (int i = 1; i < argc; i++) {
        if (!strcmp(argv[i], "--send")) send = true;
        else if (!strcmp(argv[i], "--seize")) seize = true;
        else if (!strcmp(argv[i], "--no-id-prefix")) id_prefix = false;
        else { fprintf(stderr, "unknown flag: %s\n", argv[i]); return 2; }
    }

    if (bootloader_present()) {
        printf("NOTE: 8888:cdc0 is ALREADY present — the dongle is in bootloader mode.\n");
        printf("      Power-cycle it before running this, or the result will be meaningless.\n");
        return 1;
    }

    IOHIDManagerRef mgr = IOHIDManagerCreate(kCFAllocatorDefault, kIOHIDOptionsTypeNone);
    if (!mgr) { fprintf(stderr, "IOHIDManagerCreate failed\n"); return 1; }
    IOHIDManagerSetDeviceMatching(mgr, NULL); // match everything, filter ourselves

    IOOptionBits open_opts = seize ? kIOHIDOptionsTypeSeizeDevice : kIOHIDOptionsTypeNone;
    IOReturn r = IOHIDManagerOpen(mgr, open_opts);
    if (r != kIOReturnSuccess) {
        fprintf(stderr, "IOHIDManagerOpen: 0x%08x\n", r);
        // OBSERVED on macOS 15 (Darwin 25.5.0), no dongle attached, 2026-09-05: this returns
        // 0xe00002e2 = kIOReturnNotPermitted before any device is even touched. That is TCC,
        // not USB: since macOS 10.15, opening an IOHIDManager needs Input Monitoring consent
        // for the *calling application* — for a CLI, that means the terminal app (or the
        // binary itself, if run directly). Nothing about the dongle can be learned until this
        // is granted, so it gates the whole E1/E2 experiment.
        if (r == kIOReturnNotPermitted) {
            fprintf(stderr,
                    "\n  kIOReturnNotPermitted — this is macOS TCC, not a USB problem.\n"
                    "  Grant Input Monitoring to your terminal:\n"
                    "    System Settings > Privacy & Security > Input Monitoring > + (add Terminal/iTerm)\n"
                    "  then restart the terminal and re-run. If ktflash ends up shipping an\n"
                    "  IOHIDManager-based unlock, this consent prompt becomes part of the macOS\n"
                    "  first-run experience and MUST be documented.\n");
        } else if (seize) {
            fprintf(stderr, "  (seize may additionally need root — try sudo)\n");
        }
        return 1;
    }

    CFSetRef set = IOHIDManagerCopyDevices(mgr);
    if (!set) { fprintf(stderr, "no HID devices visible\n"); return 1; }
    CFIndex n = CFSetGetCount(set);
    IOHIDDeviceRef *devs = calloc((size_t)n, sizeof(*devs));
    CFSetGetValues(set, (const void **)devs);

    IOHIDDeviceRef target = NULL;
    printf("scanning %ld HID devices…\n", (long)n);
    for (CFIndex i = 0; i < n; i++) {
        long vid = cf_num(devs[i], CFSTR(kIOHIDVendorIDKey));
        long pid = cf_num(devs[i], CFSTR(kIOHIDProductIDKey));
        long page = cf_num(devs[i], CFSTR(kIOHIDPrimaryUsagePageKey));
        if (vid != VID_KTMICRO && vid != VID_FIIO) continue;

        char product[256], mfg[256];
        cf_str(devs[i], CFSTR(kIOHIDProductKey), product, sizeof product);
        cf_str(devs[i], CFSTR(kIOHIDManufacturerKey), mfg, sizeof mfg);
        printf("  %04lx:%04lx usagePage=0x%04lx  %s / %s\n", vid, pid, page, mfg, product);

        // Prefer the vendor collection; fall back to any interface on a known VID, because
        // macOS does not always surface 0xFF01 as the *primary* usage page.
        if (page == USAGE_PAGE_VENDOR) target = devs[i];
        else if (!target) target = devs[i];
    }

    if (!target) {
        fprintf(stderr, "\nno KTMicro/FiiO HID device found. Plugged in? Try `ioreg -p IOUSB -l`.\n");
        return 1;
    }

    // Build the report. Whether IOKit wants the report ID repeated as byte 0 is exactly the
    // kind of thing that is easier to test than to argue about — hence --no-id-prefix.
    uint8_t buf[16];
    size_t len = 0;
    if (id_prefix) buf[len++] = UNLOCK_REPORT_ID;
    memcpy(buf + len, kUnlockPayload, sizeof kUnlockPayload);
    len += sizeof kUnlockPayload;

    printf("\nreport (id=0x%02x, %zu bytes):", UNLOCK_REPORT_ID, len);
    for (size_t i = 0; i < len; i++) printf(" %02x", buf[i]);
    printf("\n");

    if (!send) {
        printf("\n[dry run] nothing sent. Re-run with --send to actually attempt the unlock.\n");
        return 0;
    }

    IOReturn open_r = IOHIDDeviceOpen(target, open_opts);
    if (open_r != kIOReturnSuccess)
        printf("IOHIDDeviceOpen: 0x%08x (continuing — SetReport sometimes works anyway)\n", open_r);

    IOReturn set_r = IOHIDDeviceSetReport(target, kIOHIDReportTypeOutput, UNLOCK_REPORT_ID, buf,
                                          (CFIndex)len);
    printf("IOHIDDeviceSetReport -> 0x%08x %s\n", set_r,
           set_r == kIOReturnSuccess ? "(call succeeded)" : "(call FAILED)");

    // The call returning 0 proves nothing — docs/PROTOCOL.md's claim is precisely that the
    // call succeeds while the device ignores it. Only re-enumeration settles the question.
    printf("\nwatching for the 8888:cdc0 bootloader (5s)…\n");
    for (int i = 0; i < 25; i++) {
        if (bootloader_present()) {
            printf("\n*** PASS: bootloader appeared. Native macOS unlock WORKS. ***\n");
            printf("    docs/PROTOCOL.md's macOS caveat is wrong and should be corrected.\n");
            printf("    Next: run `ktflash flash-cdc --transport serial` (MACOS-NATIVE.md §3).\n");
            return 0;
        }
        usleep(200000);
    }

    printf("\n*** FAIL: no re-enumeration.%s ***\n",
           set_r == kIOReturnSuccess ? " The call succeeded but the device ignored it —"
                                       " consistent with docs/PROTOCOL.md."
                                     : "");
    printf("    Next, in order:\n");
    printf("      1. re-run with --no-id-prefix (report-ID framing is the likeliest mistake)\n");
    printf("      2. re-run with --seize (E2), under sudo\n");
    printf("      3. move to e3_interface_seize.c (E3) — USBInterfaceOpenSeize + WritePipe\n");
    return 1;
}
