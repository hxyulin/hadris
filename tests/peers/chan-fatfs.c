#define _POSIX_C_SOURCE 200809L
#include <dirent.h>
#define DIR FATFS_DIR
#include "ff.h"
#include "diskio.h"
#undef DIR
#include <fcntl.h>
#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

static int image_fd = -1;
static int writable;
static uint64_t image_size;
static uint64_t reads, read_bytes, writes, write_bytes, flushes, failures;

void *ff_memalloc(UINT size) { return malloc(size); }
void ff_memfree(void *ptr) { free(ptr); }
DSTATUS disk_status(BYTE drive) { return drive || image_fd < 0 ? STA_NOINIT : writable ? 0 : STA_PROTECT; }
DSTATUS disk_initialize(BYTE drive) { return disk_status(drive); }
static DRESULT transfer(BYTE drive, void *buffer, LBA_t sector, UINT count, int writing) {
    uint64_t length = (uint64_t)count * 512;
    if (drive || !count || sector > image_size / 512 || length > image_size - sector * 512 || (writing && !writable)) {
        failures++;
        return RES_PARERR;
    }
    ssize_t done = writing ? pwrite(image_fd, buffer, length, sector * 512) : pread(image_fd, buffer, length, sector * 512);
    if (done != (ssize_t)length) { failures++; return RES_ERROR; }
    return RES_OK;
}
DRESULT disk_read(BYTE drive, BYTE *buffer, LBA_t sector, UINT count) {
    reads++; read_bytes += (uint64_t)count * 512;
    return transfer(drive, buffer, sector, count, 0);
}
DRESULT disk_write(BYTE drive, const BYTE *buffer, LBA_t sector, UINT count) {
    writes++; write_bytes += (uint64_t)count * 512;
    return transfer(drive, (void *)buffer, sector, count, 1);
}
DRESULT disk_ioctl(BYTE drive, BYTE command, void *buffer) {
    if (drive) { failures++; return RES_PARERR; }
    switch (command) {
    case CTRL_SYNC:
        flushes++;
        if (writable && fsync(image_fd)) { failures++; return RES_ERROR; }
        return RES_OK;
    case GET_SECTOR_COUNT: *(LBA_t *)buffer = image_size / 512; return RES_OK;
    case GET_SECTOR_SIZE: *(WORD *)buffer = 512; return RES_OK;
    case GET_BLOCK_SIZE: *(DWORD *)buffer = 1; return RES_OK;
    default: failures++; return RES_PARERR;
    }
}
static void checked(FRESULT result) {
    if (result != FR_OK) { fprintf(stderr, "FatFs error %d\n", result); exit(1); }
}
static void host_checked(int ok) { if (!ok) { perror("host I/O"); exit(1); } }
static char *joined(const char *directory, const char *name) {
    size_t size = strlen(directory) + strlen(name) + 2;
    char *path = malloc(size);
    host_checked(path != NULL);
    snprintf(path, size, "%s/%s", directory, name);
    return path;
}
static int visible(const struct dirent *entry) { return strcmp(entry->d_name, ".") && strcmp(entry->d_name, ".."); }
static void create_files(const char *source) {
    struct dirent **entries;
    int count = scandir(source, &entries, visible, alphasort);
    host_checked(count >= 0);
    BYTE buffer[4096];
    for (int i = 0; i < count; i++) {
        char *path = joined(source, entries[i]->d_name);
        FILE *input = fopen(path, "rb");
        host_checked(input != NULL);
        FIL output;
        checked(f_open(&output, entries[i]->d_name, FA_CREATE_ALWAYS | FA_WRITE));
        size_t length;
        while ((length = fread(buffer, 1, sizeof(buffer), input))) {
            UINT written;
            checked(f_write(&output, buffer, length, &written));
            host_checked(written == length);
        }
        host_checked(!ferror(input));
        host_checked(fclose(input) == 0);
        checked(f_close(&output));
        free(path); free(entries[i]);
    }
    free(entries);
}
static void read_files(const char *destination, int extracting) {
    if (extracting) host_checked(mkdir(destination, 0700) == 0);
    FATFS_DIR directory;
    checked(f_opendir(&directory, "/"));
    for (;;) {
        FILINFO info;
        checked(f_readdir(&directory, &info));
        if (!info.fname[0]) break;
        host_checked(!(info.fattrib & AM_DIR));
        if (!extracting) { printf("/%s\n", info.fname); continue; }
        host_checked(!strchr(info.fname, '/') && !strchr(info.fname, '\\') && strcmp(info.fname, ".."));
        char *path = joined(destination, info.fname);
        FILE *output = fopen(path, "wb");
        host_checked(output != NULL);
        FIL input;
        checked(f_open(&input, info.fname, FA_READ));
        BYTE buffer[4096];
        UINT length;
        do {
            checked(f_read(&input, buffer, sizeof(buffer), &length));
            host_checked(fwrite(buffer, 1, length, output) == length);
        } while (length);
        checked(f_close(&input));
        host_checked(fclose(output) == 0);
        free(path);
    }
    checked(f_closedir(&directory));
}
int main(int argc, char **argv) {
    if (argc != 6) { fprintf(stderr, "usage: chan-fatfs WORKLOAD IMAGE BITS SIZE DIRECTORY\n"); return 2; }
    int creating = !strcmp(argv[1], "format-empty") || !strcmp(argv[1], "create-image");
    int extracting = !strcmp(argv[1], "extract-tree");
    if (!creating && !extracting && strcmp(argv[1], "list")) return 2;
    int bits = atoi(argv[3]);
    if (bits != 12 && bits != 16 && bits != 32 && bits != 64) return 2;
    writable = creating;
    image_fd = open(argv[2], creating ? O_RDWR | O_CREAT | O_TRUNC : O_RDONLY, 0600);
    host_checked(image_fd >= 0);
    if (creating) host_checked(ftruncate(image_fd, strtoull(argv[4], NULL, 10)) == 0);
    struct stat status;
    host_checked(fstat(image_fd, &status) == 0);
    image_size = status.st_size;
    FATFS fs;
    if (creating) {
        BYTE buffer[4096];
        MKFS_PARM parameters = {.fmt = (bits == 64 ? FM_EXFAT : bits == 32 ? FM_FAT32 : FM_FAT) | FM_SFD,
                                .n_fat = bits == 64 ? 1 : 2, .n_root = 512, .au_size = bits == 64 ? 4096 : bits == 12 ? 1024 : 512};
        checked(f_mkfs("", &parameters, buffer, sizeof(buffer)));
    }
    checked(f_mount(&fs, "", 1));
    if (creating) {
        checked(f_setlabel("NO NAME"));
        if (!strcmp(argv[1], "create-image")) create_files(argv[5]);
    } else read_files(argv[5], extracting);
    checked(f_unmount(""));
    host_checked(close(image_fd) == 0);
    fprintf(stderr, "IO,%" PRIu64 ",%" PRIu64 ",%" PRIu64 ",%" PRIu64 ",%" PRIu64 ",%" PRIu64 ",0\n",
            reads, read_bytes, writes, write_bytes, flushes, failures);
    return 0;
}
