# \UserSecretHandlerApi

All URIs are relative to *http://127.0.0.1:8070*

Method | HTTP request | Description
------------- | ------------- | -------------
[**create_storage_retention_handler**](UserSecretHandlerApi.md#create_storage_retention_handler) | **POST** /api/v1/user-secrets/{user_secret_id}/storage_retentions | 
[**create_user_secret_handler**](UserSecretHandlerApi.md#create_user_secret_handler) | **POST** /api/v1/user-secrets | 
[**delete_user_secret_handler**](UserSecretHandlerApi.md#delete_user_secret_handler) | **DELETE** /api/v1/user-secrets/{id} | 
[**edit_storage_retention_handler**](UserSecretHandlerApi.md#edit_storage_retention_handler) | **PATCH** /api/v1/user-secrets/{user_secret_id}/storage_retentions/{id} | 
[**edit_user_secret_handler**](UserSecretHandlerApi.md#edit_user_secret_handler) | **PATCH** /api/v1/user-secrets/{id} | 
[**get_storage_retention_handler**](UserSecretHandlerApi.md#get_storage_retention_handler) | **GET** /api/v1/user-secrets/{user_secret_id}/storage_retentions/{id} | 
[**get_user_secret_handler**](UserSecretHandlerApi.md#get_user_secret_handler) | **GET** /api/v1/user-secrets/{id} | 
[**user_secret_list_handler**](UserSecretHandlerApi.md#user_secret_list_handler) | **GET** /api/v1/user-secrets | 



## create_storage_retention_handler

> models::StorageRetentionModel create_storage_retention_handler(user_secret_id, create_storage_retention_schema)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**user_secret_id** | **uuid::Uuid** |  | [required] |
**create_storage_retention_schema** | [**CreateStorageRetentionSchema**](CreateStorageRetentionSchema.md) | create StorageRetention | [required] |

### Return type

[**models::StorageRetentionModel**](StorageRetentionModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## create_user_secret_handler

> models::UserSecretModel create_user_secret_handler(create_user_secret_schema)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**create_user_secret_schema** | [**CreateUserSecretSchema**](CreateUserSecretSchema.md) | Create user secret | [required] |

### Return type

[**models::UserSecretModel**](UserSecretModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## delete_user_secret_handler

> delete_user_secret_handler(id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | User secret id for deleting user secret | [required] |

### Return type

 (empty response body)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## edit_storage_retention_handler

> models::UserSecretModel edit_storage_retention_handler(id, user_secret_id, update_user_secret_schema)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | StorageRetention id for patching StorageRetention | [required] |
**user_secret_id** | **uuid::Uuid** |  | [required] |
**update_user_secret_schema** | [**UpdateUserSecretSchema**](UpdateUserSecretSchema.md) |  | [required] |

### Return type

[**models::UserSecretModel**](UserSecretModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## edit_user_secret_handler

> models::UserSecretModel edit_user_secret_handler(id, update_user_secret_schema)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | User secret id for patching user secret | [required] |
**update_user_secret_schema** | [**UpdateUserSecretSchema**](UpdateUserSecretSchema.md) |  | [required] |

### Return type

[**models::UserSecretModel**](UserSecretModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## get_storage_retention_handler

> models::UserSecretModel get_storage_retention_handler(id, user_secret_id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | StorageRetention id for fetching StorageRetention | [required] |
**user_secret_id** | **uuid::Uuid** |  | [required] |

### Return type

[**models::UserSecretModel**](UserSecretModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## get_user_secret_handler

> models::UserSecretModel get_user_secret_handler(id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | User secret id for fetching user secret | [required] |

### Return type

[**models::UserSecretModel**](UserSecretModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## user_secret_list_handler

> models::PagedResponseUserSecretModel user_secret_list_handler(page, limit, order_by, desc, client, name, bucket_name, user_id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**page** | Option<**i32**> | page number |  |
**limit** | Option<**i32**> | page size |  |
**order_by** | Option<**String**> | list order by |  |
**desc** | Option<**bool**> | should use descending instead |  |
**client** | Option<**String**> | client name |  |
**name** | Option<**String**> | user secret name |  |
**bucket_name** | Option<**String**> | storage bucket name |  |
**user_id** | Option<**String**> | user id of the secret owner |  |

### Return type

[**models::PagedResponseUserSecretModel**](PagedResponse_UserSecretModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

