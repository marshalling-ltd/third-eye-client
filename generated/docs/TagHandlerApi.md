# \TagHandlerApi

All URIs are relative to *http://127.0.0.1:8070*

Method | HTTP request | Description
------------- | ------------- | -------------
[**create_tag_handler**](TagHandlerApi.md#create_tag_handler) | **POST** /api/v1/tags | 
[**delete_tag_handler**](TagHandlerApi.md#delete_tag_handler) | **DELETE** /api/v1/tags/{id} | 
[**edit_tag_handler**](TagHandlerApi.md#edit_tag_handler) | **PATCH** /api/v1/tags/{id} | 
[**get_tag_handler**](TagHandlerApi.md#get_tag_handler) | **GET** /api/v1/tags/{id} | 
[**tag_list_handler**](TagHandlerApi.md#tag_list_handler) | **GET** /api/v1/tags | 
[**tag_list_search_handler**](TagHandlerApi.md#tag_list_search_handler) | **GET** /api/v1/tags/search | 



## create_tag_handler

> models::TagModel create_tag_handler(create_tag_schema)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**create_tag_schema** | [**CreateTagSchema**](CreateTagSchema.md) | Create tag | [required] |

### Return type

[**models::TagModel**](TagModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## delete_tag_handler

> delete_tag_handler(id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | Tag id for deleting tag | [required] |

### Return type

 (empty response body)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## edit_tag_handler

> models::TagModel edit_tag_handler(id, update_tag_schema)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | Tag id for patching person | [required] |
**update_tag_schema** | [**UpdateTagSchema**](UpdateTagSchema.md) | Update tag | [required] |

### Return type

[**models::TagModel**](TagModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## get_tag_handler

> models::TagModel get_tag_handler(id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | Tag id for fetching tag | [required] |

### Return type

[**models::TagModel**](TagModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## tag_list_handler

> models::PagedResponseTagModel tag_list_handler(page, limit, order_by, desc, name)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**page** | Option<**i32**> | page number |  |
**limit** | Option<**i32**> | page size |  |
**order_by** | Option<**String**> | list order by |  |
**desc** | Option<**bool**> | should use descending instead |  |
**name** | Option<**String**> | tag name |  |

### Return type

[**models::PagedResponseTagModel**](PagedResponse_TagModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## tag_list_search_handler

> models::PagedResponseTagModel tag_list_search_handler(page, limit, order_by, desc, query)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**page** | Option<**i32**> | page number |  |
**limit** | Option<**i32**> | page size |  |
**order_by** | Option<**String**> | list order by |  |
**desc** | Option<**bool**> | should use descending instead |  |
**query** | Option<**String**> | search by tag name |  |

### Return type

[**models::PagedResponseTagModel**](PagedResponse_TagModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

