# \AccountHandlerApi

All URIs are relative to *http://127.0.0.1:8070*

Method | HTTP request | Description
------------- | ------------- | -------------
[**confirm_user_handler**](AccountHandlerApi.md#confirm_user_handler) | **POST** /api/v1/account/confirm | 
[**forgot_password_handler**](AccountHandlerApi.md#forgot_password_handler) | **POST** /api/v1/account/forgot-password | 
[**login_user_handler**](AccountHandlerApi.md#login_user_handler) | **POST** /api/v1/account/login | 
[**logout_handler**](AccountHandlerApi.md#logout_handler) | **GET** /api/v1/account/logout | 
[**refresh_access_token_handler**](AccountHandlerApi.md#refresh_access_token_handler) | **POST** /api/v1/account/refresh-access-token | 
[**register_user_handler**](AccountHandlerApi.md#register_user_handler) | **POST** /api/v1/account/register | 
[**reset_password_handler**](AccountHandlerApi.md#reset_password_handler) | **POST** /api/v1/account/reset-password | 



## confirm_user_handler

> models::UserModel confirm_user_handler(register_confirm_schema)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**register_confirm_schema** | [**RegisterConfirmSchema**](RegisterConfirmSchema.md) | Confirm user | [required] |

### Return type

[**models::UserModel**](UserModel.md)

### Authorization

No authorization required

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## forgot_password_handler

> forgot_password_handler(forgot_password_schema)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**forgot_password_schema** | [**ForgotPasswordSchema**](ForgotPasswordSchema.md) | Forgot email | [required] |

### Return type

 (empty response body)

### Authorization

No authorization required

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## login_user_handler

> models::TokenResponse login_user_handler(login_schema)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**login_schema** | [**LoginSchema**](LoginSchema.md) | Login user | [required] |

### Return type

[**models::TokenResponse**](TokenResponse.md)

### Authorization

No authorization required

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## logout_handler

> logout_handler()


### Parameters

This endpoint does not need any parameter.

### Return type

 (empty response body)

### Authorization

No authorization required

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: Not defined

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## refresh_access_token_handler

> models::TokenResponse refresh_access_token_handler()


### Parameters

This endpoint does not need any parameter.

### Return type

[**models::TokenResponse**](TokenResponse.md)

### Authorization

No authorization required

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## register_user_handler

> register_user_handler(register_user_schema)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**register_user_schema** | [**RegisterUserSchema**](RegisterUserSchema.md) | Register user | [required] |

### Return type

 (empty response body)

### Authorization

No authorization required

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## reset_password_handler

> reset_password_handler(password_reset_schema)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**password_reset_schema** | [**PasswordResetSchema**](PasswordResetSchema.md) | Reset password | [required] |

### Return type

 (empty response body)

### Authorization

No authorization required

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

