# \StorageMigrationHandlerApi

All URIs are relative to *http://127.0.0.1:8070*

Method | HTTP request | Description
------------- | ------------- | -------------
[**create_storage_migration_handler**](StorageMigrationHandlerApi.md#create_storage_migration_handler) | **POST** /api/v1/storage-migrations | 
[**get_storage_migration_handler**](StorageMigrationHandlerApi.md#get_storage_migration_handler) | **GET** /api/v1/storage-migrations/{id} | 
[**list_storage_migration_handler**](StorageMigrationHandlerApi.md#list_storage_migration_handler) | **GET** /api/v1/storage-migrations | 



## create_storage_migration_handler

> models::StorageMigrationModel create_storage_migration_handler(create_storage_migration_schema)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**create_storage_migration_schema** | [**CreateStorageMigrationSchema**](CreateStorageMigrationSchema.md) | Create storage migration | [required] |

### Return type

[**models::StorageMigrationModel**](StorageMigrationModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## get_storage_migration_handler

> models::StorageMigrationModel get_storage_migration_handler(id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | Storage migration id for fetching storage migration | [required] |

### Return type

[**models::StorageMigrationModel**](StorageMigrationModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## list_storage_migration_handler

> models::PagedResponseStorageMigrationModel list_storage_migration_handler(page, limit, order_by, desc, from_storage_secret_id, to_storage_secret_id, status)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**page** | Option<**i32**> | page number |  |
**limit** | Option<**i32**> | page size |  |
**order_by** | Option<**String**> | list order by |  |
**desc** | Option<**bool**> | should use descending instead |  |
**from_storage_secret_id** | Option<**uuid::Uuid**> | Storage secret id |  |
**to_storage_secret_id** | Option<**uuid::Uuid**> | Storage secret id |  |
**status** | Option<[**StorageMigrationStatus**](.md)> | Status |  |

### Return type

[**models::PagedResponseStorageMigrationModel**](PagedResponse_StorageMigrationModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

