# \ProfileHandlerApi

All URIs are relative to *http://127.0.0.1:8070*

Method | HTTP request | Description
------------- | ------------- | -------------
[**edit_me_handler**](ProfileHandlerApi.md#edit_me_handler) | **POST** /api/v1/profile/info | 
[**edit_password_handler**](ProfileHandlerApi.md#edit_password_handler) | **PATCH** /api/v1/profile/info | 
[**get_me_handler**](ProfileHandlerApi.md#get_me_handler) | **GET** /api/v1/profile/info | 



## edit_me_handler

> models::UserModel edit_me_handler(update_user_schema)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**update_user_schema** | [**UpdateUserSchema**](UpdateUserSchema.md) |  | [required] |

### Return type

[**models::UserModel**](UserModel.md)

### Authorization

No authorization required

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## edit_password_handler

> models::UserModel edit_password_handler(password_edit_schema)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**password_edit_schema** | [**PasswordEditSchema**](PasswordEditSchema.md) |  | [required] |

### Return type

[**models::UserModel**](UserModel.md)

### Authorization

No authorization required

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## get_me_handler

> models::UserModel get_me_handler()


### Parameters

This endpoint does not need any parameter.

### Return type

[**models::UserModel**](UserModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

