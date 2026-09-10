#include "otto/buffer.h"

#include <stdint.h>
#include <stdlib.h>
#include <string.h>

void otto_buffer_init(OttoBuffer *buffer)
{
    if (buffer == NULL) {
        return;
    }

    buffer->data = NULL;
    buffer->length = 0;
    buffer->capacity = 0;
}

void otto_buffer_free(OttoBuffer *buffer)
{
    if (buffer == NULL) {
        return;
    }

    free(buffer->data);
    otto_buffer_init(buffer);
}

int otto_buffer_reserve(OttoBuffer *buffer, size_t extra)
{
    size_t required;
    size_t new_capacity;
    char *new_data;

    if (buffer == NULL || buffer->length > SIZE_MAX - 1U ||
        extra > SIZE_MAX - buffer->length - 1U) {
        return -1;
    }

    required = buffer->length + extra + 1U;
    if (required <= buffer->capacity) {
        return 0;
    }

    new_capacity = buffer->capacity == 0 ? 64U : buffer->capacity;
    while (new_capacity < required) {
        if (new_capacity > SIZE_MAX / 2U) {
            new_capacity = required;
            break;
        }
        new_capacity *= 2U;
    }

    new_data = realloc(buffer->data, new_capacity);
    if (new_data == NULL) {
        return -1;
    }

    buffer->data = new_data;
    buffer->capacity = new_capacity;
    buffer->data[buffer->length] = '\0';
    return 0;
}

int otto_buffer_append_bytes(OttoBuffer *buffer, const void *data, size_t length)
{
    if (length == 0) {
        return 0;
    }
    if (data == NULL || otto_buffer_reserve(buffer, length) != 0) {
        return -1;
    }

    memcpy(buffer->data + buffer->length, data, length);
    buffer->length += length;
    buffer->data[buffer->length] = '\0';
    return 0;
}

int otto_buffer_append_char(OttoBuffer *buffer, char value)
{
    return otto_buffer_append_bytes(buffer, &value, 1U);
}

int otto_buffer_append_cstr(OttoBuffer *buffer, const char *value)
{
    if (value == NULL) {
        return -1;
    }
    return otto_buffer_append_bytes(buffer, value, strlen(value));
}

char *otto_buffer_take(OttoBuffer *buffer)
{
    char *result;

    if (buffer == NULL) {
        return NULL;
    }

    if (buffer->data == NULL) {
        result = calloc(1U, 1U);
        if (result == NULL) {
            return NULL;
        }
    } else {
        result = buffer->data;
    }

    buffer->data = NULL;
    buffer->length = 0;
    buffer->capacity = 0;
    return result;
}
