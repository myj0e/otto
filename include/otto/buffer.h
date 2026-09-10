#ifndef OTTO_BUFFER_H
#define OTTO_BUFFER_H

#include <stddef.h>

typedef struct {
    char *data;
    size_t length;
    size_t capacity;
} OttoBuffer;

void otto_buffer_init(OttoBuffer *buffer);
void otto_buffer_free(OttoBuffer *buffer);
int otto_buffer_reserve(OttoBuffer *buffer, size_t extra);
int otto_buffer_append_bytes(OttoBuffer *buffer, const void *data, size_t length);
int otto_buffer_append_char(OttoBuffer *buffer, char value);
int otto_buffer_append_cstr(OttoBuffer *buffer, const char *value);
char *otto_buffer_take(OttoBuffer *buffer);

#endif
