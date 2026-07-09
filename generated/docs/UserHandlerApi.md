# \UserHandlerApi

All URIs are relative to *http://127.0.0.1:8070*

Method | HTTP request | Description
------------- | ------------- | -------------
[**create_user_handler**](UserHandlerApi.md#create_user_handler) | **POST** /api/v1/users | 
[**delete_user_handler**](UserHandlerApi.md#delete_user_handler) | **DELETE** /api/v1/users/{id} | 
[**edit_user_handler**](UserHandlerApi.md#edit_user_handler) | **PATCH** /api/v1/users/{id} | 
[**get_local_storage_size_handler**](UserHandlerApi.md#get_local_storage_size_handler) | **GET** /api/v1/users/{id}/local_storage_size | 
[**get_user_handler**](UserHandlerApi.md#get_user_handler) | **GET** /api/v1/users/{id} | 
[**user_list_handler**](UserHandlerApi.md#user_list_handler) | **GET** /api/v1/users | 
[**user_list_search_handler**](UserHandlerApi.md#user_list_search_handler) | **GET** /api/v1/users/search | 



## create_user_handler

> models::UserModel create_user_handler(create_user_schema)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**create_user_schema** | [**CreateUserSchema**](CreateUserSchema.md) |  | [required] |

### Return type

[**models::UserModel**](UserModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## delete_user_handler

> delete_user_handler(id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | User id for deleting user | [required] |

### Return type

 (empty response body)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## edit_user_handler

> models::UserModel edit_user_handler(id, update_user_schema)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | User id for patching user | [required] |
**update_user_schema** | [**UpdateUserSchema**](UpdateUserSchema.md) |  | [required] |

### Return type

[**models::UserModel**](UserModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## get_local_storage_size_handler

> Vec<models::StorageUse> get_local_storage_size_handler(id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | User id for fetching local storage size | [required] |

### Return type

[**Vec<models::StorageUse>**](StorageUse.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## get_user_handler

> models::UserModel get_user_handler(id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | User id for fetching user | [required] |

### Return type

[**models::UserModel**](UserModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## user_list_handler

> models::PagedResponseUserModel user_list_handler(page, limit, order_by, desc, name, pid, email, is_active)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**page** | Option<**i32**> | page number |  |
**limit** | Option<**i32**> | page size |  |
**order_by** | Option<**String**> | list order by |  |
**desc** | Option<**bool**> | should use descending instead |  |
**name** | Option<**String**> | user's name or surname |  |
**pid** | Option<**String**> | user's pid |  |
**email** | Option<**String**> | user's email |  |
**is_active** | Option<**bool**> | active/inactive users |  |

### Return type

[**models::PagedResponseUserModel**](PagedResponse_UserModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## user_list_search_handler

> models::PagedResponseUserModel user_list_search_handler(page, limit, order_by, desc, query)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**page** | Option<**i32**> | page number |  |
**limit** | Option<**i32**> | page size |  |
**order_by** | Option<**String**> | list order by |  |
**desc** | Option<**bool**> | should use descending instead |  |
**query** | Option<**String**> | search by user's name or surname, pid or email |  |

### Return type

[**models::PagedResponseUserModel**](PagedResponse_UserModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

