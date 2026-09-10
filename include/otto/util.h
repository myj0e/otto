#ifndef OTTO_UTIL_H
#define OTTO_UTIL_H

char *otto_strdup(const char *value);
char *otto_trim_copy(const char *value);
int otto_set_string(char **target, const char *value);
int otto_contains_newline(const char *value);

#endif
