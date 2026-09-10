#include "otto/mode.h"

#include <ctype.h>
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

#include "otto/config.h"
#include "otto/util.h"

#define OTTO_ACTIVE_MODE_FILENAME "active_mode"

static int path_join(const char *left, const char *right, char **result)
{
    size_t left_length;
    size_t right_length;
    size_t separator_length;
    size_t total_length;
    char *joined;

    if (left == NULL || right == NULL || result == NULL) {
        return -1;
    }

    left_length = strlen(left);
    right_length = strlen(right);
    separator_length = left_length > 0U && left[left_length - 1U] != '/';
    if (left_length > SIZE_MAX - right_length) {
        return -1;
    }
    total_length = left_length + right_length;
    if (separator_length > SIZE_MAX - total_length) {
        return -1;
    }
    total_length += separator_length;
    if (total_length == SIZE_MAX) {
        return -1;
    }

    joined = malloc(total_length + 1U);
    if (joined == NULL) {
        return -1;
    }

    memcpy(joined, left, left_length);
    if (separator_length != 0U) {
        joined[left_length++] = '/';
    }
    memcpy(joined + left_length, right, right_length + 1U);
    *result = joined;
    return 0;
}

static int valid_mode_name(const char *mode)
{
    size_t length;
    size_t index;

    if (mode == NULL || mode[0] == '\0') {
        return 0;
    }

    length = strlen(mode);
    if (length > 64U || strcmp(mode, ".") == 0 || strcmp(mode, "..") == 0) {
        return 0;
    }

    for (index = 0U; index < length; index++) {
        if (!isalnum((unsigned char)mode[index]) && mode[index] != '_' &&
            mode[index] != '-' && mode[index] != '.') {
            return 0;
        }
    }

    return 1;
}

static int directory_exists(const char *path)
{
    struct stat info;

    return stat(path, &info) == 0 && S_ISDIR(info.st_mode);
}

static int mkdir_p(const char *path)
{
    char *copy;
    char *cursor;
    size_t length;

    if (path == NULL || path[0] == '\0') {
        return -1;
    }

    if (directory_exists(path)) {
        return 0;
    }

    copy = otto_strdup(path);
    if (copy == NULL) {
        return -1;
    }

    length = strlen(copy);
    while (length > 1U && copy[length - 1U] == '/') {
        copy[--length] = '\0';
    }

    cursor = copy + (copy[0] == '/' ? 1 : 0);
    for (; *cursor != '\0'; cursor++) {
        if (*cursor != '/') {
            continue;
        }

        *cursor = '\0';
        if (copy[0] != '\0' && !directory_exists(copy)) {
            if (mkdir(copy, 0700) != 0 &&
                (errno != EEXIST || !directory_exists(copy))) {
                free(copy);
                return -1;
            }
        }
        *cursor = '/';
    }

    if (!directory_exists(copy)) {
        if (mkdir(copy, 0700) != 0 &&
            (errno != EEXIST || !directory_exists(copy))) {
            free(copy);
            return -1;
        }
    }

    free(copy);
    return 0;
}

static OttoExitCode read_mode_file(
    const char *path,
    char **content,
    int *found
)
{
    FILE *file;
    struct stat info;
    size_t expected_size;
    size_t bytes_read;
    size_t offset = 0U;
    char *data;

    *content = NULL;
    *found = 0;
    file = fopen(path, "rb");
    if (file == NULL) {
        if (errno == ENOENT) {
            return OTTO_OK;
        }
        fprintf(stderr, "otto: 无法读取模式文件 %s: %s\n", path, strerror(errno));
        return OTTO_ERR_CONFIG;
    }

    if (fstat(fileno(file), &info) != 0 || !S_ISREG(info.st_mode)) {
        fprintf(stderr, "otto: 模式文件不是普通文件：%s\n", path);
        fclose(file);
        return OTTO_ERR_CONFIG;
    }
    if (info.st_size < 0 || (unsigned long long)info.st_size > OTTO_MAX_MODE_BYTES) {
        fprintf(stderr, "otto: 模式文件超过 256 KiB 限制：%s\n", path);
        fclose(file);
        return OTTO_ERR_CONFIG;
    }

    expected_size = (size_t)info.st_size;
    data = malloc(expected_size + 1U);
    if (data == NULL) {
        fclose(file);
        return OTTO_ERR_MEMORY;
    }

    while (offset < expected_size) {
        bytes_read = fread(data + offset, 1U, expected_size - offset, file);
        if (bytes_read == 0U) {
            if (ferror(file)) {
                fprintf(stderr, "otto: 读取模式文件失败：%s\n", path);
                free(data);
                fclose(file);
                return OTTO_ERR_CONFIG;
            }
            break;
        }
        offset += bytes_read;
    }
    fclose(file);

    if (offset != expected_size) {
        fprintf(stderr, "otto: 模式文件读取不完整：%s\n", path);
        free(data);
        return OTTO_ERR_CONFIG;
    }

    data[expected_size] = '\0';
    if (expected_size >= 3U &&
        (unsigned char)data[0] == 0xEFU &&
        (unsigned char)data[1] == 0xBBU &&
        (unsigned char)data[2] == 0xBFU) {
        expected_size -= 3U;
        memmove(data, data + 3U, expected_size);
        data[expected_size] = '\0';
    }

    *content = data;
    *found = 1;
    return OTTO_OK;
}

static OttoExitCode read_active_mode_file(
    const char *path,
    char **mode,
    int *found
)
{
    FILE *file;
    struct stat info;
    size_t expected_size;
    size_t offset = 0U;
    size_t bytes_read;
    char *data;
    char *trimmed;

    *mode = NULL;
    *found = 0;
    file = fopen(path, "rb");
    if (file == NULL) {
        if (errno == ENOENT) {
            return OTTO_OK;
        }
        fprintf(stderr, "otto: 无法读取当前模式文件 %s: %s\n", path, strerror(errno));
        return OTTO_ERR_CONFIG;
    }

    if (fstat(fileno(file), &info) != 0 || !S_ISREG(info.st_mode)) {
        fprintf(stderr, "otto: 当前模式文件不是普通文件：%s\n", path);
        fclose(file);
        return OTTO_ERR_CONFIG;
    }
    if (info.st_size < 0 || (unsigned long long)info.st_size > 64U) {
        fprintf(stderr, "otto: 当前模式文件无效：%s\n", path);
        fclose(file);
        return OTTO_ERR_CONFIG;
    }

    expected_size = (size_t)info.st_size;
    data = malloc(expected_size + 1U);
    if (data == NULL) {
        fclose(file);
        return OTTO_ERR_MEMORY;
    }

    while (offset < expected_size) {
        bytes_read = fread(data + offset, 1U, expected_size - offset, file);
        if (bytes_read == 0U) {
            if (ferror(file)) {
                fprintf(stderr, "otto: 读取当前模式文件失败：%s\n", path);
                free(data);
                fclose(file);
                return OTTO_ERR_CONFIG;
            }
            break;
        }
        offset += bytes_read;
    }
    fclose(file);

    if (offset != expected_size) {
        fprintf(stderr, "otto: 当前模式文件读取不完整：%s\n", path);
        free(data);
        return OTTO_ERR_CONFIG;
    }

    data[expected_size] = '\0';
    trimmed = otto_trim_copy(data);
    free(data);
    if (trimmed == NULL) {
        return OTTO_ERR_MEMORY;
    }
    if (trimmed[0] == '\0') {
        free(trimmed);
        return OTTO_OK;
    }
    if (!valid_mode_name(trimmed)) {
        fprintf(stderr, "otto: 当前模式名称无效：%s\n", trimmed);
        free(trimmed);
        return OTTO_ERR_CONFIG;
    }

    *mode = trimmed;
    *found = 1;
    return OTTO_OK;
}

static OttoExitCode get_active_mode_path(char **path)
{
    char *directory = NULL;
    OttoExitCode code;

    if (path == NULL) {
        return OTTO_ERR_USAGE;
    }
    *path = NULL;

    code = otto_config_get_directory(&directory);
    if (code != OTTO_OK) {
        return code;
    }
    if (path_join(directory, OTTO_ACTIVE_MODE_FILENAME, path) != 0) {
        free(directory);
        return OTTO_ERR_MEMORY;
    }
    free(directory);
    return OTTO_OK;
}

static OttoExitCode write_active_mode_file(
    const char *path,
    const char *mode
)
{
    char *temporary_path = NULL;
    size_t path_length;
    size_t temporary_size;
    int file_descriptor = -1;
    FILE *file = NULL;
    OttoExitCode code = OTTO_ERR_CONFIG;

    path_length = strlen(path);
    if (path_length > SIZE_MAX - sizeof(".tmp.XXXXXX")) {
        return OTTO_ERR_MEMORY;
    }
    temporary_size = path_length + sizeof(".tmp.XXXXXX");
    temporary_path = malloc(temporary_size);
    if (temporary_path == NULL) {
        return OTTO_ERR_MEMORY;
    }
    (void)snprintf(temporary_path, temporary_size, "%s.tmp.XXXXXX", path);

    file_descriptor = mkstemp(temporary_path);
    if (file_descriptor < 0) {
        fprintf(stderr, "otto: 无法创建当前模式临时文件：%s\n", strerror(errno));
        goto failure;
    }
    if (fchmod(file_descriptor, 0600) != 0) {
        fprintf(stderr, "otto: 无法设置当前模式文件权限：%s\n", strerror(errno));
        goto failure;
    }

    file = fdopen(file_descriptor, "w");
    if (file == NULL) {
        fprintf(stderr, "otto: 无法打开当前模式临时文件：%s\n", strerror(errno));
        goto failure;
    }
    file_descriptor = -1;

    if (mode != NULL && (fputs(mode, file) == EOF || fputc('\n', file) == EOF)) {
        fprintf(stderr, "otto: 写入当前模式失败：%s\n", strerror(errno));
        goto failure;
    }
    if (fflush(file) != 0 || fsync(fileno(file)) != 0) {
        fprintf(stderr, "otto: 写入当前模式失败：%s\n", strerror(errno));
        goto failure;
    }
    if (fclose(file) != 0) {
        file = NULL;
        fprintf(stderr, "otto: 关闭当前模式文件失败：%s\n", strerror(errno));
        goto failure;
    }
    file = NULL;

    if (rename(temporary_path, path) != 0) {
        fprintf(stderr, "otto: 保存当前模式失败：%s\n", strerror(errno));
        goto failure;
    }
    (void)chmod(path, 0600);
    code = OTTO_OK;

failure:
    if (file != NULL) {
        fclose(file);
    } else if (file_descriptor >= 0) {
        close(file_descriptor);
    }
    if (code != OTTO_OK) {
        unlink(temporary_path);
    }
    free(temporary_path);
    return code;
}

OttoExitCode otto_mode_get_active(char **mode, int *found)
{
    char *path = NULL;
    OttoExitCode code;

    if (mode == NULL || found == NULL) {
        return OTTO_ERR_USAGE;
    }
    *mode = NULL;
    *found = 0;

    code = get_active_mode_path(&path);
    if (code == OTTO_OK) {
        code = read_active_mode_file(path, mode, found);
    }
    free(path);
    return code;
}

OttoExitCode otto_mode_set_active(const char *mode)
{
    char *directory = NULL;
    char *path = NULL;
    OttoExitCode code;

    if (mode != NULL && mode[0] != '\0' && !valid_mode_name(mode)) {
        fprintf(stderr, "otto: 模式名称无效：%s\n", mode);
        return OTTO_ERR_USAGE;
    }
    if (mode != NULL && mode[0] == '\0') {
        mode = NULL;
    }

    code = otto_config_get_directory(&directory);
    if (code != OTTO_OK) {
        return code;
    }
    if (mkdir_p(directory) != 0) {
        fprintf(stderr, "otto: 无法创建模式配置目录 %s：%s\n", directory, strerror(errno));
        free(directory);
        return OTTO_ERR_CONFIG;
    }
    if (path_join(directory, OTTO_ACTIVE_MODE_FILENAME, &path) != 0) {
        free(directory);
        return OTTO_ERR_MEMORY;
    }
    free(directory);

    code = write_active_mode_file(path, mode);
    free(path);
    return code;
}

static OttoExitCode load_from_directory(
    const char *directory,
    const char *mode,
    char **system_prompt,
    int *found,
    char **attempted_path
)
{
    char *filename = NULL;
    char *path = NULL;
    OttoExitCode code;

    filename = malloc(strlen(mode) + sizeof(".md"));
    if (filename == NULL) {
        return OTTO_ERR_MEMORY;
    }
    (void)snprintf(filename, strlen(mode) + sizeof(".md"), "%s.md", mode);

    if (path_join(directory, filename, &path) != 0) {
        free(filename);
        return OTTO_ERR_MEMORY;
    }
    free(filename);

    if (attempted_path != NULL) {
        free(*attempted_path);
        *attempted_path = otto_strdup(path);
        if (*attempted_path == NULL) {
            free(path);
            return OTTO_ERR_MEMORY;
        }
    }

    code = read_mode_file(path, system_prompt, found);
    free(path);
    return code;
}

OttoExitCode otto_mode_load(
    const char *mode,
    char **system_prompt,
    int *found
)
{
    const char *mode_directory_override;
    char *directory = NULL;
    char *attempted_path = NULL;
    int explicit_directory;
    OttoExitCode code;

    if (system_prompt == NULL || found == NULL ||
        !valid_mode_name(mode)) {
        fprintf(stderr, "otto: 模式名称无效：%s\n", mode == NULL ? "" : mode);
        return OTTO_ERR_USAGE;
    }

    *system_prompt = NULL;
    *found = 0;
    mode_directory_override = getenv("OTTO_MODE_DIR");
    explicit_directory = mode_directory_override != NULL &&
        mode_directory_override[0] != '\0';

    if (explicit_directory) {
        directory = otto_strdup(mode_directory_override);
        code = directory == NULL ? OTTO_ERR_MEMORY : OTTO_OK;
    } else {
        code = otto_config_get_directory(&directory);
    }
    if (code != OTTO_OK) {
        return code;
    }

    code = load_from_directory(
        directory,
        mode,
        system_prompt,
        found,
        &attempted_path
    );
    if (code != OTTO_OK || *found || explicit_directory ||
        strcmp(mode, "otto") != 0) {
        if (code == OTTO_OK && !*found) {
            fprintf(stderr, "otto: 找不到模式文件：%s\n", attempted_path);
            code = OTTO_ERR_CONFIG;
        }
        free(attempted_path);
        free(directory);
        return code;
    }

    /* While developing from the source tree, allow the bundled otto.md. */
    free(attempted_path);
    attempted_path = NULL;
    code = load_from_directory(".", mode, system_prompt, found, &attempted_path);
    if (code == OTTO_OK && *found) {
        free(attempted_path);
        free(directory);
        return OTTO_OK;
    }

    free(attempted_path);
    free(directory);
    return code;
}
