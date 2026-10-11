#pragma once
#include <windows.h>
void nativeInit(UINT64 hash);
void nativePush64(UINT64 value);
PUINT64 nativeCall();
int worldGetAllPeds(int* peds,int count);
BYTE* getScriptHandleBaseAddress(int handle);
