#include <errno.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/uio.h>
int main(int argc,char **argv) {
    if(argc!=2)return 2;
    int fd=open(argv[1],O_RDONLY); char a[64],b[16];
    if(fd<0 || read(fd,a,64)!=64 || pread(fd,b,16,0)!=16)return 1;
    struct iovec vec[2]={{a,8},{b,8}};
    if(readv(fd,vec,2)!=16 || lseek(fd,0,SEEK_SET)!=0)return 1;
    int other=open("/dev/zero",O_RDONLY);
    if(other<0 || read(other,a,32)!=32 || close(other))return 1;
    if(pread(fd,a,64,-1)!=-1 || errno!=EINVAL)return 1;
    return close(fd)!=0;
}
