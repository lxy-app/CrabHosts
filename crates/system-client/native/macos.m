#import <Foundation/Foundation.h>
#import <ServiceManagement/ServiceManagement.h>
#import <Security/Security.h>
#import <dispatch/dispatch.h>
#import <stdbool.h>
#import <stdint.h>
#import <stdlib.h>
#import <string.h>
#import <unistd.h>
#import <xpc/xpc.h>

#define CH_SERVICE_NAME "dev.crabhosts.hosts-helper"
#define CH_PLIST_NAME "dev.crabhosts.hosts-helper.plist"
#define CH_MAX_REQUEST_BYTES (8 * 1024 * 1024)

typedef char *(*CHRequestHandler)(const char *request);

static char *CHCopyUTF8(NSString *value) {
    if (value == nil) {
        return NULL;
    }
    const char *bytes = value.UTF8String;
    return bytes == NULL ? NULL : strdup(bytes);
}

static void CHSetError(char **error, NSString *message) {
    if (error != NULL && *error == NULL) {
        *error = CHCopyUTF8(message);
    }
}

static NSString *CHXPCDescription(xpc_object_t object) {
    if (xpc_get_type(object) == XPC_TYPE_ERROR) {
        const char *message = xpc_dictionary_get_string(object, XPC_ERROR_KEY_DESCRIPTION);
        if (message != NULL) {
            return [NSString stringWithUTF8String:message];
        }
    }
    char *description = xpc_copy_description(object);
    NSString *result = [NSString stringWithUTF8String:description];
    free(description);
    return result;
}

static void CHSetNSError(char **error, NSError *nativeError) {
    if (nativeError == nil) {
        CHSetError(error, @"Unknown ServiceManagement error");
        return;
    }
    NSString *message = [NSString stringWithFormat: @"%@ (%@ %ld)",
        nativeError.localizedDescription, nativeError.domain, (long)nativeError.code];
    CHSetError(error, message);
}

static SMAppService *CHHelperService(void) API_AVAILABLE(macos(13.0)) {
    return [SMAppService daemonServiceWithPlistName:@CH_PLIST_NAME];
}

int crabhosts_helper_registration_status(void) {
    if (@available(macOS 13.0, *)) {
        return (int)CHHelperService().status;
    }
    return -1;
}

int crabhosts_helper_register(char **error) {
    if (error != NULL) {
        *error = NULL;
    }
    if (@available(macOS 13.0, *)) {
        SMAppService *service = CHHelperService();
        SMAppServiceStatus initialStatus = service.status;
        NSError *nativeError = nil;
        if (initialStatus == SMAppServiceStatusEnabled) {
            // Explicit repair/update action. Replacing the executable requires re-registration.
            if (![service unregisterAndReturnError:&nativeError]) {
                CHSetNSError(error, nativeError);
                return -1;
            }
        }
        if (![service registerAndReturnError:&nativeError]) {
            CHSetNSError(error, nativeError);
            return -1;
        }
        return (int)service.status;
    }
    CHSetError(error, @"SMAppService requires macOS 13 or newer");
    return -1;
}

bool crabhosts_process_is_root(void) {
    return geteuid() == 0;
}

void crabhosts_native_string_free(char *value) {
    free(value);
}

char *crabhosts_xpc_send(const char *request, const char *helperRequirement, char **error) {
    if (error != NULL) {
        *error = NULL;
    }
    if (request == NULL || helperRequirement == NULL) {
        CHSetError(error, @"XPC request and signing requirement are required");
        return NULL;
    }

    __block char *result = NULL;
    __block char *replyError = NULL;
    __block bool abandoned = false;
    dispatch_semaphore_t finished = dispatch_semaphore_create(0);
    dispatch_queue_t replyQueue = dispatch_queue_create(
        "dev.crabhosts.hosts-helper.client", DISPATCH_QUEUE_SERIAL);
    xpc_connection_t connection = xpc_connection_create_mach_service(
        CH_SERVICE_NAME, replyQueue, XPC_CONNECTION_MACH_SERVICE_PRIVILEGED);

    int requirementStatus = xpc_connection_set_peer_code_signing_requirement(
        connection, helperRequirement);
    if (requirementStatus != 0) {
        CHSetError(error, [NSString stringWithFormat:
            @"Cannot set Helper signing requirement (error %d)", requirementStatus]);
        xpc_connection_cancel(connection);
        return NULL;
    }

    xpc_connection_set_event_handler(connection, ^(xpc_object_t event) {
        (void)event;
    });
    xpc_connection_activate(connection);

    xpc_object_t message = xpc_dictionary_create(NULL, NULL, 0);
    xpc_dictionary_set_string(message, "request", request);
    xpc_connection_send_message_with_reply(connection, message, replyQueue, ^(xpc_object_t reply) {
        // A timed-out caller must never receive a late write through a stale error pointer.
        if (abandoned) {
            return;
        }
        if (xpc_get_type(reply) == XPC_TYPE_ERROR) {
            CHSetError(&replyError, CHXPCDescription(reply));
        } else if (xpc_get_type(reply) != XPC_TYPE_DICTIONARY) {
            CHSetError(&replyError, @"Helper returned a non-dictionary XPC response");
        } else {
            const char *response = xpc_dictionary_get_string(reply, "response");
            if (response == NULL) {
                CHSetError(&replyError, @"Helper response is missing its response field");
            } else if (strnlen(response, 65 * 1024) > 64 * 1024) {
                CHSetError(&replyError, @"Helper response exceeds the 64 KiB limit");
            } else {
                result = strdup(response);
            }
        }
        dispatch_semaphore_signal(finished);
    });

    long waitResult = dispatch_semaphore_wait(
        finished, dispatch_time(DISPATCH_TIME_NOW, 15 * NSEC_PER_SEC));
    if (waitResult != 0) {
        // Serialize with an in-flight reply, then mark the byref state abandoned before cancel.
        // A reply that arrives later only reads live block state and performs no allocation.
        dispatch_sync(replyQueue, ^{
            abandoned = true;
        });
        xpc_connection_cancel(connection);
        free(result);
        free(replyError);
        CHSetError(error, @"Timed out waiting for the CrabHosts Helper; write outcome is unknown");
        return NULL;
    }

    xpc_connection_cancel(connection);
    if (result == NULL && replyError != NULL) {
        if (error != NULL && *error == NULL) {
            *error = replyError;
            replyError = NULL;
        }
    }
    free(replyError);
    if (result == NULL) {
        CHSetError(error, @"Helper returned an empty response");
    }
    return result;
}

static bool CHValidateMessageSender(xpc_object_t message, const char *requirementString) {
    SecCodeRef code = NULL;
    SecRequirementRef requirement = NULL;
    CFStringRef requirementText = CFStringCreateWithCString(
        kCFAllocatorDefault, requirementString, kCFStringEncodingUTF8);
    if (requirementText == NULL) {
        return false;
    }
    OSStatus status = SecRequirementCreateWithString(
        requirementText, kSecCSDefaultFlags, &requirement);
    CFRelease(requirementText);
    if (status != errSecSuccess) {
        return false;
    }
    status = SecCodeCreateWithXPCMessage(message, kSecCSDefaultFlags, &code);
    if (status == errSecSuccess) {
        status = SecCodeCheckValidityWithErrors(code, kSecCSDefaultFlags, requirement, NULL);
    }
    if (code != NULL) {
        CFRelease(code);
    }
    if (requirement != NULL) {
        CFRelease(requirement);
    }
    return status == errSecSuccess;
}

static void CHSendError(xpc_connection_t peer, xpc_object_t request,
    const char *code, const char *message) {
    NSString *json = [NSString stringWithFormat:
        @"{\"error\":{\"code\":\"%s\",\"message\":\"%s\"}}", code, message];
    xpc_object_t reply = xpc_dictionary_create_reply(request);
    if (reply == NULL) {
        return;
    }
    xpc_dictionary_set_string(reply, "response", json.UTF8String);
    xpc_connection_send_message(peer, reply);
}

int crabhosts_helper_serve(const char *clientRequirement, CHRequestHandler handler,
    char **error) {
    if (error != NULL) {
        *error = NULL;
    }
    if (clientRequirement == NULL || handler == NULL) {
        CHSetError(error, @"Client signing requirement and request handler are required");
        return -1;
    }
    if (@available(macOS 12.0, *)) {
        xpc_connection_t listener = xpc_connection_create_mach_service(
            CH_SERVICE_NAME, dispatch_get_main_queue(), XPC_CONNECTION_MACH_SERVICE_LISTENER);
        xpc_connection_set_event_handler(listener, ^(xpc_object_t event) {
            if (xpc_get_type(event) != XPC_TYPE_CONNECTION) {
                return;
            }
            xpc_connection_t peer = (xpc_connection_t)event;
            int requirementStatus = xpc_connection_set_peer_code_signing_requirement(
                peer, clientRequirement);
            if (requirementStatus != 0) {
                xpc_connection_cancel(peer);
                return;
            }
            xpc_connection_set_event_handler(peer, ^(xpc_object_t message) {
                if (xpc_get_type(message) != XPC_TYPE_DICTIONARY) {
                    return;
                }
                // Validate the audit token attached to this exact message, in addition to the
                // connection-level peer requirement. Never use PID as a security identity.
                if (!CHValidateMessageSender(message, clientRequirement)) {
                    CHSendError(peer, message, "permission_denied", "client signature validation failed");
                    return;
                }
                const char *request = xpc_dictionary_get_string(message, "request");
                if (request == NULL) {
                    CHSendError(peer, message, "helper_protocol", "missing request field");
                    return;
                }
                size_t requestLength = strnlen(request, CH_MAX_REQUEST_BYTES + 1);
                if (requestLength > CH_MAX_REQUEST_BYTES) {
                    CHSendError(peer, message, "helper_protocol", "request exceeds size limit");
                    return;
                }

                char *response = handler(request);
                if (response == NULL) {
                    CHSendError(peer, message, "helper_protocol", "request handler returned no response");
                    return;
                }
                xpc_object_t reply = xpc_dictionary_create_reply(message);
                if (reply != NULL) {
                    xpc_dictionary_set_string(reply, "response", response);
                    xpc_connection_send_message(peer, reply);
                }
                free(response);
            });
            xpc_connection_activate(peer);
        });
        xpc_connection_activate(listener);
        dispatch_main();
        return 0;
    }
    CHSetError(error, @"The CrabHosts Helper requires macOS 13 or newer");
    return -1;
}
