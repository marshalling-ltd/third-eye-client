# \SearchHandlerApi

All URIs are relative to *http://127.0.0.1:8070*

Method | HTTP request | Description
------------- | ------------- | -------------
[**search_resources_handler**](SearchHandlerApi.md#search_resources_handler) | **POST** /api/v1/search | 



## search_resources_handler

> models::SearchResourceResponseModel search_resources_handler(search_resources_schema)


Search AOIs, POIs, and intermagnet analyses by resource type, tags, and optional location radius.

### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**search_resources_schema** | [**SearchResourcesSchema**](SearchResourcesSchema.md) | Search filters | [required] |

### Return type

[**models::SearchResourceResponseModel**](SearchResourceResponseModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

