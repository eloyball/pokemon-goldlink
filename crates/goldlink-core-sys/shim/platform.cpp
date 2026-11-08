// SPDX-License-Identifier: GPL-3.0-or-later
//
// melonDS's platform layer (Platform.h), which a frontend must define, on the standard library.
// GoldLink gives the core its ROM and save from memory, so files, wireless, the network, the
// camera, the microphone and add-ons are minimal or no-ops: the virtual link runs through the
// ROM's own rings in RAM (CLK1), not melonDS's multiplayer interface (RFC §7 Q4).

#include <atomic>
#include <chrono>
#include <condition_variable>
#include <cstdarg>
#include <cstdio>
#include <mutex>
#include <string>
#include <thread>

#include "goldlink_core.h"
#include "Platform.h"
#include "SPI_Firmware.h"

void gl_core_on_save(void *userdata, const uint8_t *save, uint32_t save_len, uint32_t offset, uint32_t len);

static std::atomic<GlLogCallback> g_log {nullptr};

extern "C" void gl_set_log_callback(GlLogCallback cb)
{
    g_log.store(cb);
}

namespace melonDS::Platform
{

void SignalStop(StopReason reason, void *userdata)
{
    Log(Info, "melonDS stopped (reason %d)\n", static_cast<int>(reason));
}

// ---------------------------------------------------------------------------------------------
// Files (FileHandle is a FILE*)

static FILE *F(FileHandle *f) { return reinterpret_cast<FILE *>(f); }

std::string GetLocalFilePath(const std::string &filename) { return filename; }

FileHandle *OpenFile(const std::string &path, FileMode mode)
{
    const char *m;
    bool text = (mode & Text) != 0;
    if ((mode & ReadWrite) == ReadWrite)
    {
        if (mode & Preserve)
        {
            FILE *probe = std::fopen(path.c_str(), "rb");
            if (probe == nullptr && (mode & NoCreate))
                return nullptr;
            if (probe != nullptr)
                std::fclose(probe);
            else if (FILE *created = std::fopen(path.c_str(), "wb"))
                std::fclose(created);
            m = text ? "r+" : "r+b";
        }
        else
            m = text ? "w+" : "w+b";
    }
    else if (mode & Append)
        m = text ? "a" : "ab";
    else if (mode & Write)
        m = text ? "w" : "wb";
    else if (mode & Read)
        m = text ? "r" : "rb";
    else
        return nullptr;
    return reinterpret_cast<FileHandle *>(std::fopen(path.c_str(), m));
}

FileHandle *OpenLocalFile(const std::string &path, FileMode mode) { return OpenFile(path, mode); }

bool FileExists(const std::string &name)
{
    if (FILE *f = std::fopen(name.c_str(), "rb"))
    {
        std::fclose(f);
        return true;
    }
    return false;
}

bool LocalFileExists(const std::string &name) { return FileExists(name); }

bool CheckFileWritable(const std::string &filepath)
{
    if (FILE *f = std::fopen(filepath.c_str(), "ab"))
    {
        std::fclose(f);
        return true;
    }
    return false;
}

bool CheckLocalFileWritable(const std::string &filepath) { return CheckFileWritable(filepath); }

bool CloseFile(FileHandle *file) { return std::fclose(F(file)) == 0; }
bool IsEndOfFile(FileHandle *file) { return std::feof(F(file)) != 0; }
bool FileReadLine(char *str, int count, FileHandle *file) { return std::fgets(str, count, F(file)) != nullptr; }
u64 FilePosition(FileHandle *file) { return static_cast<u64>(std::ftell(F(file))); }

bool FileSeek(FileHandle *file, s64 offset, FileSeekOrigin origin)
{
    int whence = origin == FileSeekOrigin::Start ? SEEK_SET : origin == FileSeekOrigin::Current ? SEEK_CUR : SEEK_END;
    return std::fseek(F(file), static_cast<long>(offset), whence) == 0;
}

void FileRewind(FileHandle *file) { std::rewind(F(file)); }
u64 FileRead(void *data, u64 size, u64 count, FileHandle *file) { return std::fread(data, size, count, F(file)); }
bool FileFlush(FileHandle *file) { return std::fflush(F(file)) == 0; }
u64 FileWrite(const void *data, u64 size, u64 count, FileHandle *file) { return std::fwrite(data, size, count, F(file)); }

u64 FileWriteFormatted(FileHandle *file, const char *fmt, ...)
{
    va_list args;
    va_start(args, fmt);
    int n = std::vfprintf(F(file), fmt, args);
    va_end(args);
    return n > 0 ? static_cast<u64>(n) : 0;
}

u64 FileLength(FileHandle *file)
{
    long pos = std::ftell(F(file));
    std::fseek(F(file), 0, SEEK_END);
    long len = std::ftell(F(file));
    std::fseek(F(file), pos, SEEK_SET);
    return len > 0 ? static_cast<u64>(len) : 0;
}

// ---------------------------------------------------------------------------------------------
// Log

void Log(LogLevel level, const char *fmt, ...)
{
    GlLogCallback cb = g_log.load();
    if (cb == nullptr)
        return;
    char buf[1024];
    va_list args;
    va_start(args, fmt);
    std::vsnprintf(buf, sizeof(buf), fmt, args);
    va_end(args);
    cb(static_cast<int>(level), buf);
}

// ---------------------------------------------------------------------------------------------
// Threads, semaphores, mutexes, time

struct Thread
{
    std::thread t;
};

Thread *Thread_Create(std::function<void()> func) { return new Thread {std::thread(std::move(func))}; }

void Thread_Free(Thread *thread)
{
    if (thread->t.joinable())
        thread->t.detach();
    delete thread;
}

void Thread_Wait(Thread *thread)
{
    if (thread->t.joinable())
        thread->t.join();
}

struct Semaphore
{
    std::mutex m;
    std::condition_variable cv;
    int count = 0;
};

Semaphore *Semaphore_Create() { return new Semaphore; }
void Semaphore_Free(Semaphore *sema) { delete sema; }

void Semaphore_Reset(Semaphore *sema)
{
    std::lock_guard<std::mutex> lock(sema->m);
    sema->count = 0;
}

void Semaphore_Wait(Semaphore *sema)
{
    std::unique_lock<std::mutex> lock(sema->m);
    sema->cv.wait(lock, [sema] { return sema->count > 0; });
    sema->count--;
}

bool Semaphore_TryWait(Semaphore *sema, int timeout_ms)
{
    std::unique_lock<std::mutex> lock(sema->m);
    if (timeout_ms <= 0)
    {
        if (sema->count == 0)
            return false;
    }
    else if (!sema->cv.wait_for(lock, std::chrono::milliseconds(timeout_ms), [sema] { return sema->count > 0; }))
        return false;
    sema->count--;
    return true;
}

void Semaphore_Post(Semaphore *sema, int count)
{
    {
        std::lock_guard<std::mutex> lock(sema->m);
        sema->count += count;
    }
    sema->cv.notify_all();
}

struct Mutex
{
    std::mutex m;
};

Mutex *Mutex_Create() { return new Mutex; }
void Mutex_Free(Mutex *mutex) { delete mutex; }
void Mutex_Lock(Mutex *mutex) { mutex->m.lock(); }
void Mutex_Unlock(Mutex *mutex) { mutex->m.unlock(); }
bool Mutex_TryLock(Mutex *mutex) { return mutex->m.try_lock(); }

void Sleep(u64 usecs) { std::this_thread::sleep_for(std::chrono::microseconds(usecs)); }

static const auto g_start = std::chrono::steady_clock::now();

u64 GetMSCount()
{
    return std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now() - g_start).count();
}

u64 GetUSCount()
{
    return std::chrono::duration_cast<std::chrono::microseconds>(std::chrono::steady_clock::now() - g_start).count();
}

// ---------------------------------------------------------------------------------------------
// Saves, firmware, clock

void WriteNDSSave(const u8 *savedata, u32 savelen, u32 writeoffset, u32 writelen, void *userdata)
{
    gl_core_on_save(userdata, savedata, savelen, writeoffset, writelen);
}

void WriteGBASave(const u8 *, u32, u32, u32, void *) {}
void WriteFirmware(const Firmware &, u32, u32, void *) {}  // generated firmware: nothing to keep
void WriteDateTime(int, int, int, int, int, int, void *) {} // the clock follows the host's

// ---------------------------------------------------------------------------------------------
// Local wireless, network, camera, microphone, AAC, add-ons, dynamic libraries: unused

void MP_Begin(void *) {}
void MP_End(void *) {}
int MP_SendPacket(u8 *, int, u64, void *) { return 0; }
int MP_RecvPacket(u8 *, u64 *, void *) { return 0; }
int MP_SendCmd(u8 *, int, u64, void *) { return 0; }
int MP_SendReply(u8 *, int, u64, u16, void *) { return 0; }
int MP_SendAck(u8 *, int, u64, void *) { return 0; }
int MP_RecvHostPacket(u8 *, u64 *, void *) { return 0; }
u16 MP_RecvReplies(u8 *, u64, u16, void *) { return 0; }

int Net_SendPacket(u8 *, int, void *) { return 0; }
int Net_RecvPacket(u8 *, void *) { return 0; }

void Camera_Start(int, void *) {}
void Camera_Stop(int, void *) {}
void Camera_CaptureFrame(int, u32 *, int, int, bool, void *) {}

void Mic_Start(void *) {}
void Mic_Stop(void *) {}
int Mic_ReadInput(s16 *, int, void *) { return 0; }

AACDecoder *AAC_Init() { return nullptr; }
void AAC_DeInit(AACDecoder *) {}
bool AAC_Configure(AACDecoder *, int, int) { return false; }
bool AAC_DecodeFrame(AACDecoder *, const void *, int, void *, int) { return false; }

bool Addon_KeyDown(KeyType, void *) { return false; }
void Addon_RumbleStart(u32, void *) {}
void Addon_RumbleStop(void *) {}
float Addon_MotionQuery(MotionQueryType, void *) { return 0.0f; }

DynamicLibrary *DynamicLibrary_Load(const char *) { return nullptr; }
void DynamicLibrary_Unload(DynamicLibrary *) {}
void *DynamicLibrary_LoadFunction(DynamicLibrary *, const char *) { return nullptr; }

} // namespace melonDS::Platform
