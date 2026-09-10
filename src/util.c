#include "otto/util.h"

#include <ctype.h>
#include <stdlib.h>
#include <string.h>

char *otto_strdup(const char *value)
{
    size_t length;
    char *copy;

    if (value == NULL) {
        return NULL;
    }

    length = strlen(value);
    copy = malloc(length + 1U);
    if (copy == NULL) {
        return NULL;
    }

    memcpy(copy, value, length + 1U);
    return copy;
}

char *otto_trim_copy(const char *value)
{
    const unsigned char *start;
    const unsigned char *end;
    size_t length;
    char *copy;

    if (value == NULL) {
        return NULL;
    }

    start = (const unsigned char *)value;
    while (*start != '\0' && isspace(*start)) {
        start++;
    }

    end = start + strlen((const char *)start);
    while (end > start && isspace(end[-1])) {
        end--;
    }

    length = (size_t)(end - start);
    copy = malloc(length + 1U);
    if (copy == NULL) {
        return NULL;
    }

    memcpy(copy, start, length);
    copy[length] = '\0';
    return copy;
}

int otto_set_string(char **target, const char *value)
{
    char *copy;

    if (target == NULL || value == NULL) {
        return -1;
    }

    copy = otto_strdup(value);
    if (copy == NULL) {
        return -1;
    }

    free(*target);
    *target = copy;
    return 0;
}

int otto_contains_newline(const char *value)
{
    if (value == NULL) {
        return 0;
    }

    return strchr(value, '\n') != NULL || strchr(value, '\r') != NULL;
}
